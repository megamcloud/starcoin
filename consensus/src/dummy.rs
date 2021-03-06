// Copyright (c) The Starcoin Core Contributors
// SPDX-License-Identifier: Apache-2.0

use anyhow::{Error, Result};
use config::NodeConfig;
use logger::prelude::*;
use rand::prelude::*;
use std::convert::TryFrom;
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use traits::ChainReader;
use traits::{Consensus, ConsensusHeader};
use types::block::BlockHeader;
use types::U256;

//TODO add some field to DummyHeader.
#[derive(Clone, Debug)]
pub struct DummyHeader {}

impl ConsensusHeader for DummyHeader {}

impl TryFrom<Vec<u8>> for DummyHeader {
    type Error = Error;

    fn try_from(_value: Vec<u8>) -> Result<Self> {
        Ok(DummyHeader {})
    }
}

impl Into<Vec<u8>> for DummyHeader {
    fn into(self) -> Vec<u8> {
        vec![]
    }
}

#[derive(Clone)]
pub struct DummyConsensus {}

impl Consensus for DummyConsensus {
    type ConsensusHeader = DummyHeader;

    fn calculate_next_difficulty(config: Arc<NodeConfig>, _reader: &dyn ChainReader) -> U256 {
        let mut rng = rand::thread_rng();
        // if produce block on demand, use a default wait time.
        let high: u64 = if config.miner.dev_period == 0 {
            1000
        } else {
            config.miner.dev_period * 1000
        };
        let time: u64 = rng.gen_range(1, high);
        time.into()
    }

    fn solve_consensus_header(_header_hash: &[u8], difficulty: U256) -> Self::ConsensusHeader {
        let time: u64 = difficulty.as_u64();
        debug!("DummyConsensus rand sleep time : {}", time);
        thread::sleep(Duration::from_millis(time));
        DummyHeader {}
    }

    fn verify_header(
        _config: Arc<NodeConfig>,
        _reader: &dyn ChainReader,
        _header: &BlockHeader,
    ) -> Result<()> {
        Ok(())
    }
}
