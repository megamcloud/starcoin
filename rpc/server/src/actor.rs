// Copyright (c) The Starcoin Core Contributors
// SPDX-License-Identifier: Apache-2

use crate::module::{
    ChainRpcImpl, DebugRpcImpl, NodeRpcImpl, StateRpcImpl, TxPoolRpcImpl, WalletRpcImpl,
};
use crate::service::RpcService;
use actix::prelude::*;
use anyhow::Result;
use jsonrpc_core::IoHandler;
use starcoin_config::NodeConfig;
use starcoin_logger::prelude::*;
use starcoin_logger::LoggerHandle;
use starcoin_network::NetworkAsyncService;
use starcoin_rpc_api::chain::ChainApi;
use starcoin_rpc_api::debug::DebugApi;
use starcoin_rpc_api::wallet::WalletApi;
use starcoin_rpc_api::{node::NodeApi, state::StateApi, txpool::TxPoolApi};
use starcoin_state_api::ChainStateAsyncService;
use starcoin_traits::ChainAsyncService;
use starcoin_txpool_api::TxPoolAsyncService;
use starcoin_wallet_api::WalletAsyncService;
use std::sync::Arc;

pub struct RpcActor {
    config: Arc<NodeConfig>,
    io_handler: IoHandler,
    server: Option<RpcService>,
}

impl RpcActor {
    pub fn launch<CS, TS, AS, SS>(
        config: Arc<NodeConfig>,
        txpool_service: TS,
        chain_service: CS,
        account_service: AS,
        state_service: SS,
        //TODO after network async service provide trait, remove Option.
        network_service: Option<NetworkAsyncService>,
        logger_handle: Option<Arc<LoggerHandle>>,
    ) -> Result<(Addr<RpcActor>, IoHandler)>
    where
        CS: ChainAsyncService + 'static,
        TS: TxPoolAsyncService + 'static,
        AS: WalletAsyncService + 'static,
        SS: ChainStateAsyncService + 'static,
    {
        Self::launch_with_apis(
            config.clone(),
            NodeRpcImpl::new(config.clone(), network_service),
            Some(ChainRpcImpl::new(chain_service)),
            Some(TxPoolRpcImpl::new(txpool_service)),
            Some(WalletRpcImpl::new(account_service)),
            Some(StateRpcImpl::new(state_service)),
            logger_handle.map(|logger_handle| DebugRpcImpl::new(config, logger_handle)),
        )
    }

    pub fn launch_with_apis<C, N, T, A, S, D>(
        config: Arc<NodeConfig>,
        node_api: N,
        chain_api: Option<C>,
        txpool_api: Option<T>,
        account_api: Option<A>,
        state_api: Option<S>,
        debug_api: Option<D>,
    ) -> Result<(Addr<Self>, IoHandler)>
    where
        N: NodeApi,
        C: ChainApi,
        T: TxPoolApi,
        A: WalletApi,
        S: StateApi,
        D: DebugApi,
    {
        let mut io_handler = IoHandler::new();
        io_handler.extend_with(NodeApi::to_delegate(node_api));
        if let Some(chain_api) = chain_api {
            io_handler.extend_with(ChainApi::to_delegate(chain_api));
        }
        if let Some(txpool_api) = txpool_api {
            io_handler.extend_with(TxPoolApi::to_delegate(txpool_api));
        }
        if let Some(account_api) = account_api {
            io_handler.extend_with(WalletApi::to_delegate(account_api));
        }
        if let Some(state_api) = state_api {
            io_handler.extend_with(StateApi::to_delegate(state_api));
        }
        if let Some(debug_api) = debug_api {
            io_handler.extend_with(DebugApi::to_delegate(debug_api));
        }
        Self::launch_with_handler(config, io_handler)
    }

    pub fn launch_with_handler(
        config: Arc<NodeConfig>,
        io_handler: IoHandler,
    ) -> Result<(Addr<Self>, IoHandler)> {
        let actor = RpcActor {
            config,
            server: None,
            io_handler: io_handler.clone(),
        };
        Ok((actor.start(), io_handler))
    }

    fn do_start(&mut self) {
        let server = RpcService::new(self.config.clone(), self.io_handler.clone());
        self.server = Some(server);
    }

    fn do_stop(&mut self) {
        let server = std::mem::replace(&mut self.server, None);
        match server {
            Some(server) => server.close(),
            None => {}
        }
    }
}

impl Actor for RpcActor {
    type Context = Context<Self>;

    fn started(&mut self, _ctx: &mut Self::Context) {
        self.do_start();
    }

    fn stopping(&mut self, _ctx: &mut Self::Context) -> Running {
        self.do_stop();
        Running::Stop
    }
}

impl Supervised for RpcActor {
    fn restarting(&mut self, _ctx: &mut Self::Context) {
        info!("Restart JSON rpc service.");
        self.do_stop();
        self.do_start();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use starcoin_chain::mock::mock_chain_service::MockChainService;
    use starcoin_state_api::mock::MockChainStateService;
    use starcoin_txpool_mock_service::MockTxPoolService;
    use starcoin_wallet_api::mock::MockWalletService;

    #[stest::test]
    async fn test_start() {
        let logger_handle = starcoin_logger::init_for_test();
        let config = Arc::new(NodeConfig::random_for_test());
        let txpool = MockTxPoolService::new();
        let account_service = MockWalletService::new().unwrap();
        let state_service = MockChainStateService::new();
        let chain_service = MockChainService::new();
        let _rpc_actor = RpcActor::launch(
            config,
            txpool,
            chain_service,
            account_service,
            state_service,
            None,
            Some(logger_handle),
        )
        .unwrap();
    }
}
