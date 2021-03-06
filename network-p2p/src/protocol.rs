pub mod event;
pub mod generic_proto;
pub mod message;
pub mod rpc_handle;
pub mod util;

use crate::config::ProtocolId;
use crate::protocol::generic_proto::{GenericProto, GenericProtoOut};
use crate::utils::interval;
use crate::{DiscoveryNetBehaviour, Multiaddr};

use crate::network_state::Peer;

use bytes::{Bytes, BytesMut};
use futures::prelude::*;
use libp2p::core::{nodes::listeners::ListenerId, ConnectedPoint};
use libp2p::swarm::{IntoProtocolsHandler, ProtocolsHandler};
use libp2p::swarm::{NetworkBehaviour, NetworkBehaviourAction, PollParameters};
use libp2p::PeerId;
use log::Level;

use crate::protocol::message::generic::{ConsensusMessage, Message, Status};
use crypto::HashValue;
use scs::SCSCodec;
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::pin::Pin;
use std::str;
use std::sync::Arc;
use std::task::Poll;
use std::time;
use types::peer_info::PeerInfo;
use wasm_timer::Instant;

const REQUEST_TIMEOUT_SEC: u64 = 40;
/// Interval at which we perform time based maintenance
const TICK_TIMEOUT: time::Duration = time::Duration::from_millis(1100);
/// Current protocol version.
pub(crate) const CURRENT_VERSION: u32 = 1;
/// Lowest version we support
pub(crate) const MIN_VERSION: u32 = 1;

mod rep {
    use peerset::ReputationChange as Rep;
    /// Reputation change when a peer is "clogged", meaning that it's not fast enough to process our
    /// messages.
    pub const CLOGGED_PEER: Rep = Rep::new(-(1 << 12), "Clogged message queue");
    /// Reputation change when a peer doesn't respond in time to our messages.
    pub const TIMEOUT: Rep = Rep::new(-(1 << 10), "Request timeout");
    /// Reputation change when a peer sends us a status message while we already received one.
    pub const UNEXPECTED_STATUS: Rep = Rep::new(-(1 << 20), "Unexpected status message");
    /// Reputation change when we are a light client and a peer is behind us.
    pub const PEER_BEHIND_US_LIGHT: Rep = Rep::new(-(1 << 8), "Useless for a light peer");
    /// Reputation change when a peer sends us an extrinsic that we didn't know about.
    pub const GOOD_EXTRINSIC: Rep = Rep::new(1 << 7, "Good extrinsic");
    /// Reputation change when a peer sends us a bad extrinsic.
    pub const BAD_EXTRINSIC: Rep = Rep::new(-(1 << 12), "Bad extrinsic");
    /// We sent an RPC query to the given node, but it failed.
    pub const RPC_FAILED: Rep = Rep::new(-(1 << 12), "Remote call failed");
    /// We received a message that failed to decode.
    pub const BAD_MESSAGE: Rep = Rep::new(-(1 << 12), "Bad message");
    /// We received an unexpected response.
    pub const UNEXPECTED_RESPONSE: Rep = Rep::new_fatal("Unexpected response packet");
    /// We received an unexpected extrinsic packet.
    pub const UNEXPECTED_EXTRINSICS: Rep = Rep::new_fatal("Unexpected extrinsics packet");
    /// We received an unexpected light node request.
    pub const UNEXPECTED_REQUEST: Rep = Rep::new_fatal("Unexpected block request packet");
    /// Peer has different genesis.
    pub const GENESIS_MISMATCH: Rep = Rep::new_fatal("Genesis mismatch");
    /// Peer is on unsupported protocol version.
    pub const BAD_PROTOCOL: Rep = Rep::new_fatal("Unsupported protocol");
    /// Peer role does not match (e.g. light peer connecting to another light peer).
    pub const BAD_ROLE: Rep = Rep::new_fatal("Unsupported role");
    /// Peer response data does not have requested bits.
    pub const BAD_RESPONSE: Rep = Rep::new(-(1 << 12), "Incomplete response");
}

#[derive(Debug)]
pub enum CustomMessageOutcome {
    NotificationStreamOpened {
        remote: PeerId,
        info: PeerInfo,
    },
    /// Notification protocols have been closed with a remote.
    NotificationStreamClosed {
        remote: PeerId,
    },
    /// Messages have been received on one or more notifications protocols.
    NotificationsReceived {
        remote: PeerId,
        messages: Vec<Bytes>,
    },
    None,
}

/// A peer that we are connected to
/// and from whom we have not yet received a Status message.
struct HandshakingPeer {
    timestamp: Instant,
}

#[derive(Default)]
struct PacketStats {
    bytes_in: u64,
    bytes_out: u64,
    count_in: u64,
    count_out: u64,
}

struct ContextData {
    // All connected peers
    peers: HashMap<PeerId, Peer>,
}

pub struct ChainInfo {
    pub genesis_hash: HashValue,
    pub self_info: PeerInfo,
}

pub struct Protocol {
    /// Interval at which we call `tick`.
    tick_timeout: Pin<Box<dyn Stream<Item = ()> + Send>>,
    important_peers: HashSet<PeerId>,
    /// Connected peers pending Status message.
    handshaking_peers: HashMap<PeerId, HandshakingPeer>,
    /// Used to report reputation changes.
    peerset_handle: peerset::PeersetHandle,
    /// Handles opening the unique substream and sending and receiving raw messages.
    behaviour: GenericProto,
    context_data: ContextData,
    /// The `PeerId`'s of all boot nodes.
    boot_node_ids: Arc<HashSet<PeerId>>,

    chain_info: ChainInfo,
}

impl NetworkBehaviour for Protocol {
    type ProtocolsHandler = <GenericProto as NetworkBehaviour>::ProtocolsHandler;
    type OutEvent = CustomMessageOutcome;

    fn new_handler(&mut self) -> Self::ProtocolsHandler {
        self.behaviour.new_handler()
    }

    fn addresses_of_peer(&mut self, peer_id: &PeerId) -> Vec<Multiaddr> {
        self.behaviour.addresses_of_peer(peer_id)
    }

    fn inject_connected(&mut self, peer_id: PeerId, endpoint: ConnectedPoint) {
        self.behaviour.inject_connected(peer_id, endpoint)
    }

    fn inject_disconnected(&mut self, peer_id: &PeerId, endpoint: ConnectedPoint) {
        self.behaviour.inject_disconnected(peer_id, endpoint)
    }

    fn inject_node_event(
        &mut self,
        peer_id: PeerId,
        event: <<Self::ProtocolsHandler as IntoProtocolsHandler>::Handler as ProtocolsHandler>::OutEvent,
    ) {
        self.behaviour.inject_node_event(peer_id, event)
    }

    fn poll(
        &mut self,
        cx: &mut std::task::Context,
        params: &mut impl PollParameters,
    ) -> Poll<
        NetworkBehaviourAction<
            <<Self::ProtocolsHandler as IntoProtocolsHandler>::Handler as ProtocolsHandler>::InEvent,
            Self::OutEvent
        >
>{
        while let Poll::Ready(Some(())) = self.tick_timeout.poll_next_unpin(cx) {
            self.tick();
        }

        let event = match self.behaviour.poll(cx, params) {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(NetworkBehaviourAction::GenerateEvent(ev)) => ev,
            Poll::Ready(NetworkBehaviourAction::DialAddress { address }) => {
                return Poll::Ready(NetworkBehaviourAction::DialAddress { address })
            }
            Poll::Ready(NetworkBehaviourAction::DialPeer { peer_id }) => {
                return Poll::Ready(NetworkBehaviourAction::DialPeer { peer_id })
            }
            Poll::Ready(NetworkBehaviourAction::SendEvent { peer_id, event }) => {
                return Poll::Ready(NetworkBehaviourAction::SendEvent { peer_id, event })
            }
            Poll::Ready(NetworkBehaviourAction::ReportObservedAddr { address }) => {
                return Poll::Ready(NetworkBehaviourAction::ReportObservedAddr { address })
            }
        };

        let outcome = match event {
            GenericProtoOut::CustomProtocolOpen { peer_id, .. } => {
                self.on_peer_connected(peer_id);
                CustomMessageOutcome::None
            }
            GenericProtoOut::CustomProtocolClosed { peer_id, .. } => {
                self.on_peer_disconnected(peer_id.clone());
                // Notify all the notification protocols as closed.
                CustomMessageOutcome::NotificationStreamClosed { remote: peer_id }
            }
            GenericProtoOut::CustomMessage { peer_id, message } => {
                self.on_custom_message(peer_id, message)
            }
            GenericProtoOut::Clogged {
                peer_id: _,
                messages,
            } => {
                debug!(target: "sync", "{} clogging messages:", messages.len());
                for _msg in messages.into_iter().take(5) {
                    //let message: Option<Message<B>> = Decode::decode(&mut &msg[..]).ok();
                    //debug!(target: "sync", "{:?}", message);
                    //self.on_clogged_peer(peer_id.clone(), message);
                }
                CustomMessageOutcome::None
            }
        };

        if let CustomMessageOutcome::None = outcome {
            Poll::Pending
        } else {
            Poll::Ready(NetworkBehaviourAction::GenerateEvent(outcome))
        }
    }

    fn inject_replaced(
        &mut self,
        peer_id: PeerId,
        closed_endpoint: ConnectedPoint,
        new_endpoint: ConnectedPoint,
    ) {
        self.behaviour
            .inject_replaced(peer_id, closed_endpoint, new_endpoint)
    }

    fn inject_addr_reach_failure(
        &mut self,
        peer_id: Option<&PeerId>,
        addr: &Multiaddr,
        error: &dyn std::error::Error,
    ) {
        self.behaviour
            .inject_addr_reach_failure(peer_id, addr, error)
    }

    fn inject_dial_failure(&mut self, peer_id: &PeerId) {
        self.behaviour.inject_dial_failure(peer_id)
    }

    fn inject_new_listen_addr(&mut self, addr: &Multiaddr) {
        self.behaviour.inject_new_listen_addr(addr)
    }

    fn inject_expired_listen_addr(&mut self, addr: &Multiaddr) {
        self.behaviour.inject_expired_listen_addr(addr)
    }

    fn inject_new_external_addr(&mut self, addr: &Multiaddr) {
        self.behaviour.inject_new_external_addr(addr)
    }

    fn inject_listener_error(&mut self, id: ListenerId, err: &(dyn std::error::Error + 'static)) {
        self.behaviour.inject_listener_error(id, err);
    }

    fn inject_listener_closed(&mut self, id: ListenerId) {
        self.behaviour.inject_listener_closed(id);
    }
}

impl DiscoveryNetBehaviour for Protocol {
    fn add_discovered_nodes(&mut self, peer_ids: impl Iterator<Item = PeerId>) {
        self.behaviour.add_discovered_nodes(peer_ids)
    }
}

// impl Drop for Protocol {
//     fn drop(&mut self) {
//         //debug!(target: "sync", "Network stats:\n{}", self.format_stats());
//     }
// }

impl Protocol {
    /// Create a new instance.
    pub fn new(
        peerset_config: peerset::PeersetConfig,
        protocol_id: ProtocolId,
        chain_info: ChainInfo,
        boot_node_ids: Arc<HashSet<PeerId>>,
    ) -> crate::net_error::Result<(Protocol, peerset::PeersetHandle)> {
        let important_peers = {
            let mut imp_p = HashSet::new();
            for reserved in &peerset_config.reserved_nodes {
                imp_p.insert(reserved.clone());
            }
            imp_p.shrink_to_fit();
            imp_p
        };

        let (peerset, peerset_handle) = peerset::Peerset::from_config(peerset_config);
        let versions = &((MIN_VERSION as u8)..=(CURRENT_VERSION as u8)).collect::<Vec<u8>>();
        let behaviour = GenericProto::new(protocol_id, versions, peerset);

        let protocol = Protocol {
            tick_timeout: Box::pin(interval(TICK_TIMEOUT)),
            handshaking_peers: HashMap::new(),
            important_peers,
            peerset_handle: peerset_handle.clone(),
            behaviour,
            context_data: ContextData {
                peers: HashMap::new(),
            },
            chain_info,
            boot_node_ids,
        };

        Ok((protocol, peerset_handle))
    }

    /// Returns the list of all the peers we have an open channel to.
    pub fn open_peers(&self) -> impl Iterator<Item = &PeerId> {
        self.behaviour.open_peers()
    }

    /// Returns true if we have a channel open with this node.
    pub fn is_open(&self, peer_id: &PeerId) -> bool {
        self.behaviour.is_open(peer_id)
    }

    /// Disconnects the given peer if we are connected to it.
    pub fn disconnect_peer(&mut self, peer_id: &PeerId) {
        self.behaviour.disconnect_peer(peer_id)
    }

    /// Returns true if we try to open protocols with the given peer.
    pub fn is_enabled(&self, peer_id: &PeerId) -> bool {
        self.behaviour.is_enabled(peer_id)
    }

    /// Returns the state of the peerset manager, for debugging purposes.
    pub fn peerset_debug_info(&mut self) -> serde_json::Value {
        self.behaviour.peerset_debug_info()
    }

    pub fn on_custom_message(&mut self, who: PeerId, data: BytesMut) -> CustomMessageOutcome {
        trace!("receive custom message from {} ", who);
        let message = match Message::decode(&data[..]) {
            Ok(message) => message,
            Err(err) => {
                info!(target: "sync", "Couldn't decode packet sent by {}: {:?}: {}", who, data, err);
                self.peerset_handle.report_peer(who, rep::BAD_MESSAGE);
                return CustomMessageOutcome::None;
            }
        };

        match message {
            Message::Consensus(msg) => CustomMessageOutcome::NotificationsReceived {
                remote: who,
                messages: vec![Bytes::from(msg.data)],
            },
            Message::Status(status) => self.on_status_message(who, status),
        }
    }

    /// Called by peer to report status
    fn on_status_message(&mut self, who: PeerId, status: Status) -> CustomMessageOutcome {
        trace!(target: "sync", "New peer {} {:?}", who, status);
        let _protocol_version = {
            if self.context_data.peers.contains_key(&who) {
                log!(
                    target: "sync",
                    if self.important_peers.contains(&who) { Level::Warn } else { Level::Debug },
                    "Unexpected status packet from {}", who
                );
                self.peerset_handle.report_peer(who, rep::UNEXPECTED_STATUS);
                return CustomMessageOutcome::None;
            }
            if status.genesis_hash != self.chain_info.genesis_hash {
                info!(
                    "Peer is on different chain (our genesis: {} theirs: {})",
                    self.chain_info.genesis_hash, status.genesis_hash
                );
                self.peerset_handle
                    .report_peer(who.clone(), rep::GENESIS_MISMATCH);
                self.behaviour.disconnect_peer(&who);

                if self.boot_node_ids.contains(&who) {
                    error!(
                        target: "sync",
                        "Bootnode with peer id `{}` is on a different chain (our genesis: {} theirs: {})",
                        who,
                        self.chain_info.genesis_hash,
                        status.genesis_hash,
                    );
                }

                return CustomMessageOutcome::None;
            }
            if status.version < MIN_VERSION && CURRENT_VERSION < status.min_supported_version {
                log!(
                    target: "sync",
                    if self.important_peers.contains(&who) { Level::Warn } else { Level::Trace },
                    "Peer {:?} using unsupported protocol version {}", who, status.version
                );
                self.peerset_handle
                    .report_peer(who.clone(), rep::BAD_PROTOCOL);
                self.behaviour.disconnect_peer(&who);
                return CustomMessageOutcome::None;
            }

            match self.handshaking_peers.remove(&who) {
                Some(_handshaking) => {}
                None => {
                    error!(target: "sync", "Received status from previously unconnected node {}", who);
                    return CustomMessageOutcome::None;
                }
            };

            debug!(target: "sync", "Connected {}", who);
            status.version
        };
        // Notify all the notification protocols as open.
        CustomMessageOutcome::NotificationStreamOpened {
            remote: who,
            info: status.info,
        }
    }

    fn send_message(&mut self, who: &PeerId, message: Message) {
        send_message(&mut self.behaviour, who, message);
    }

    /// Called when a new peer is connected
    pub fn on_peer_connected(&mut self, who: PeerId) {
        info!(target: "sync", "Connecting {}", who);
        self.handshaking_peers.insert(
            who.clone(),
            HandshakingPeer {
                timestamp: Instant::now(),
            },
        );
        self.send_status(who);
    }

    /// Send Status message
    fn send_status(&mut self, who: PeerId) {
        let status = message::generic::Status {
            version: CURRENT_VERSION,
            min_supported_version: MIN_VERSION,
            genesis_hash: self.chain_info.genesis_hash,
            info: self.chain_info.self_info.clone(),
        };

        self.send_message(&who, Message::Status(status))
    }

    /// Called by peer when it is disconnecting
    pub fn on_peer_disconnected(&mut self, peer: PeerId) {
        if self.important_peers.contains(&peer) {
            warn!(target: "sync", "Reserved peer {} disconnected", peer);
        } else {
            trace!(target: "sync", "{} disconnected", peer);
        }

        // lock all the the peer lists so that add/remove peer events are in order
        {
            self.handshaking_peers.remove(&peer);
        };
    }

    /// Called as a back-pressure mechanism if the networking detects that the peer cannot process
    /// our messaging rate fast enough.
    pub fn on_clogged_peer(&self, who: PeerId) {
        self.peerset_handle.report_peer(who, rep::CLOGGED_PEER);
    }

    /// Adjusts the reputation of a node.
    pub fn report_peer(&self, who: PeerId, reputation: peerset::ReputationChange) {
        self.peerset_handle.report_peer(who, reputation)
    }

    /// Perform time based maintenance.
    ///
    /// > **Note**: This method normally doesn't have to be called except for testing purposes.
    pub fn tick(&mut self) {
        self.maintain_peers();
    }

    fn maintain_peers(&mut self) {
        let tick = Instant::now();
        let mut aborting = Vec::new();
        {
            for (who, _) in self.handshaking_peers.iter().filter(|(_, handshaking)| {
                (tick - handshaking.timestamp).as_secs() > REQUEST_TIMEOUT_SEC
            }) {
                info!(
                    target: "sync",
                    "Handshake timeout {}", who
                );
                aborting.push(who.clone());
            }
        }

        for p in aborting {
            self.behaviour.disconnect_peer(&p);
            self.peerset_handle.report_peer(p, rep::TIMEOUT);
        }
    }

    /// Send a notification to the given peer we're connected to.
    ///
    /// Doesn't do anything if we don't have a notifications substream for that protocol with that
    /// peer.
    pub fn write_notification(
        &mut self,
        target: PeerId,
        _protocol_name: Cow<'static, [u8]>,
        message: impl Into<Vec<u8>>,
    ) {
        self.send_message(
            &target,
            Message::Consensus(ConsensusMessage {
                data: message.into(),
            }),
        );
        // self.behaviour
        //     .write_notification(&target, protocol_name, message);
    }

    pub fn register_notifications_protocol(
        &mut self,
        protocol_name: impl Into<Cow<'static, [u8]>>,
    ) -> Vec<event::Event> {
        let protocol_name = protocol_name.into();
        self.behaviour
            .register_notif_protocol(protocol_name.clone(), Vec::new());

        info!("register protocol {:?}", str::from_utf8(&protocol_name));
        self.context_data
            .peers
            .iter()
            .map(|(peer_id, _peer)| event::Event::NotificationStreamOpened {
                remote: peer_id.clone(),
                info: self.chain_info.self_info.clone(),
            })
            .collect()
    }

    pub fn update_self_info(&mut self, self_info: PeerInfo) {
        self.chain_info.self_info = self_info;
    }
}

fn send_message(
    behaviour: &mut GenericProto,
    who: &PeerId,
    message: Message,
) -> anyhow::Result<()> {
    let encoded = message.encode()?;
    behaviour.send_packet(who, encoded);
    Ok(())
}
