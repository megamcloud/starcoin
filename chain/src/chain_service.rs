// Copyright (c) The Starcoin Core Contributors
// SPDX-License-Identifier: Apache-2.0

use crate::chain::BlockChain;
use crate::chain_state_store::ChainStateStore;
use crate::message::{ChainRequest, ChainResponse};
use actix::prelude::*;
use anyhow::{Error, Result};
use config::NodeConfig;
use consensus::{Consensus, ConsensusHeader};
use crypto::{hash::CryptoHash, HashValue};
use executor::TransactionExecutor;
use futures_locks::RwLock;
use std::cell::RefCell;
use std::marker::PhantomData;
use std::sync::Arc;
use storage::{memory_storage::MemoryStorage, StarcoinStorage};
use traits::{ChainReader, ChainService, ChainStateReader, ChainWriter};
use types::{
    account_address::AccountAddress,
    block::{Block, BlockHeader, BlockNumber, BlockTemplate},
    transaction::{SignedUserTransaction, Transaction, TransactionInfo, TransactionStatus},
};

pub struct ChainServiceImpl<E, C>
where
    E: TransactionExecutor,
    C: Consensus,
{
    config: Arc<NodeConfig>,
    head: BlockChain<E, C>,
    branches: Vec<BlockChain<E, C>>,
    storage: Arc<StarcoinStorage>,
}

impl<E, C> ChainServiceImpl<E, C>
where
    E: TransactionExecutor,
    C: Consensus,
{
    pub fn new(config: Arc<NodeConfig>, storage: Arc<StarcoinStorage>) -> Result<Self> {
        let latest_header = storage.block_store.get_latest_block_header()?;
        let head = BlockChain::new(
            config.clone(),
            storage.clone(),
            latest_header.map(|header| header.id()),
        )?;
        let branches = Vec::new();
        Ok(Self {
            config,
            head,
            branches,
            storage,
        })
    }

    pub fn find_or_fork(&mut self, header: &BlockHeader) -> BlockChain<E, C> {
        unimplemented!()
    }

    pub fn state_at(&self, root: HashValue) -> ChainStateStore {
        unimplemented!()
    }

    fn select_head(&mut self) {
        //select head branch;
        todo!()
    }
}

impl<E, C> ChainService for ChainServiceImpl<E, C>
where
    E: TransactionExecutor,
    C: Consensus,
{
    //TODO define connect result.
    fn try_connect(&mut self, block: Block) -> Result<()> {
        let header = block.header();
        let mut branch = self.find_or_fork(&header);
        branch.apply(block)?;
        self.select_head();
        todo!()
    }
}

impl<E, C> ChainReader for ChainServiceImpl<E, C>
where
    E: TransactionExecutor,
    C: Consensus,
{
    fn head_block(&self) -> Block {
        self.head.head_block()
    }

    fn current_header(&self) -> BlockHeader {
        self.head.current_header()
    }

    fn get_header(&self, hash: HashValue) -> Result<Option<BlockHeader>> {
        self.head.get_header(hash)
    }

    fn get_header_by_number(&self, number: u64) -> Result<Option<BlockHeader>> {
        self.head.get_header_by_number(number)
    }

    fn get_block_by_number(&self, number: u64) -> Result<Option<Block>> {
        self.head.get_block_by_number(number)
    }

    fn get_block(&self, hash: HashValue) -> Result<Option<Block>> {
        self.head.get_block(hash)
    }

    fn get_transaction(&self, hash: HashValue) -> Result<Option<Transaction>> {
        self.head.get_transaction(hash)
    }

    fn get_transaction_info(&self, hash: HashValue) -> Result<Option<TransactionInfo>> {
        self.head.get_transaction_info(hash)
    }

    fn create_block_template(&self, txns: Vec<SignedUserTransaction>) -> Result<BlockTemplate> {
        self.head.create_block_template(txns)
    }

    fn chain_state_reader(&self) -> &dyn ChainStateReader {
        self.head.chain_state_reader()
    }
}