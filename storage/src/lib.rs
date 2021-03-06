// Copyright (c) The Starcoin Core Contributors
// SPDX-License-Identifier: Apache-2.0

use crate::accumulator::AccumulatorStorage;
use crate::block::BlockStorage;
use crate::block_info::{BlockInfoStorage, BlockInfoStore};
use crate::state_node::StateStorage;
use crate::storage::{ColumnFamilyName, InnerStorage, KVStore, StorageInstance};
use crate::transaction::TransactionStorage;
use crate::transaction_info::TransactionInfoStorage;
use anyhow::{ensure, Error, Result};
use crypto::HashValue;
use once_cell::sync::Lazy;
use starcoin_accumulator::{
    AccumulatorNode, AccumulatorReader, AccumulatorTreeStore, AccumulatorWriter,
};
use starcoin_types::transaction::Transaction;
use starcoin_types::{
    block::{Block, BlockBody, BlockHeader, BlockInfo},
    startup_info::StartupInfo,
    transaction::TransactionInfo,
};
use state_tree::{StateNode, StateNodeStore};
use std::collections::BTreeMap;
use std::convert::TryInto;
use std::sync::Arc;

pub mod accumulator;
pub mod batch;
pub mod block;
pub mod block_info;
pub mod cache_storage;
pub mod db_storage;
pub mod state_node;
pub mod storage;
#[cfg(test)]
mod tests;
pub mod transaction;
pub mod transaction_info;

#[macro_use]
pub mod storage_macros;

pub const ACCUMULATOR_NODE_PREFIX_NAME: ColumnFamilyName = "acc_node";
pub const BLOCK_PREFIX_NAME: ColumnFamilyName = "block";
pub const BLOCK_HEADER_PREFIX_NAME: ColumnFamilyName = "block_header";
pub const BLOCK_SONS_PREFIX_NAME: ColumnFamilyName = "block_sons";
pub const BLOCK_BODY_PREFIX_NAME: ColumnFamilyName = "block_body";
pub const BLOCK_NUM_PREFIX_NAME: ColumnFamilyName = "block_num";
pub const BLOCK_INFO_PREFIX_NAME: ColumnFamilyName = "block_info";
pub const BLOCK_TRANSATIONS_PREFIX_NAME: ColumnFamilyName = "block_txns";
pub const STATE_NODE_PREFIX_NAME: ColumnFamilyName = "state_node";
pub const STARTUP_INFO_PREFIX_NAME: ColumnFamilyName = "startup_info";
pub const TRANSACTION_PREFIX_NAME: ColumnFamilyName = "transaction";
pub const TRANSACTION_INFO_PREFIX_NAME: ColumnFamilyName = "transaction_info";
///db storage use prefix_name vec to init
/// Please note that adding a prefix needs to be added in vec simultaneously, remember！！
pub static VEC_PREFIX_NAME: Lazy<Vec<ColumnFamilyName>> = Lazy::new(|| {
    vec![
        ACCUMULATOR_NODE_PREFIX_NAME,
        BLOCK_PREFIX_NAME,
        BLOCK_HEADER_PREFIX_NAME,
        BLOCK_SONS_PREFIX_NAME,
        BLOCK_BODY_PREFIX_NAME,
        BLOCK_NUM_PREFIX_NAME,
        BLOCK_INFO_PREFIX_NAME,
        BLOCK_TRANSATIONS_PREFIX_NAME,
        STATE_NODE_PREFIX_NAME,
        STARTUP_INFO_PREFIX_NAME,
        TRANSACTION_PREFIX_NAME,
        TRANSACTION_INFO_PREFIX_NAME,
    ]
});

pub trait BlockStore {
    fn get_startup_info(&self) -> Result<Option<StartupInfo>>;
    fn save_startup_info(&self, startup_info: StartupInfo) -> Result<()>;

    fn get_headers(&self) -> Result<Vec<HashValue>>;

    fn save_branch_number(
        &self,
        branch_id: HashValue,
        number: u64,
        block_id: HashValue,
    ) -> Result<()>;

    fn get_block(&self, block_id: HashValue) -> Result<Option<Block>>;

    fn get_body(&self, block_id: HashValue) -> Result<Option<BlockBody>>;

    fn get_branch_number(&self, branch_id: HashValue, number: u64) -> Result<Option<HashValue>>;

    fn get_number(&self, number: u64) -> Result<Option<HashValue>>;

    fn commit_branch_block(&self, branch_id: HashValue, block: Block) -> Result<()>;

    fn get_branch_hashes(&self, block_id: HashValue) -> Result<Vec<HashValue>>;

    fn get_latest_block_header(&self) -> Result<Option<BlockHeader>>;

    fn get_latest_block(&self) -> Result<Block>;

    fn get_block_header_by_hash(&self, block_id: HashValue) -> Result<Option<BlockHeader>>;

    fn get_block_by_hash(&self, block_id: HashValue) -> Result<Option<Block>>;

    fn get_block_header_by_number(&self, number: u64) -> Result<Option<BlockHeader>>;

    fn get_header_by_branch_number(
        &self,
        branch_id: HashValue,
        number: u64,
    ) -> Result<Option<BlockHeader>>;

    fn get_block_by_number(&self, number: u64) -> Result<Option<Block>>;

    fn get_block_by_branch_number(
        &self,
        branch_id: HashValue,
        number: u64,
    ) -> Result<Option<Block>>;

    fn get_common_ancestor(
        &self,
        block_id1: HashValue,
        block_id2: HashValue,
    ) -> Result<Option<HashValue>>;
    fn get_block_transactions(&self, block_id: HashValue) -> Result<Vec<HashValue>>;
    fn save_block_transactions(
        &self,
        block_id: HashValue,
        transactions: Vec<HashValue>,
    ) -> Result<()>;
}

pub trait TransactionInfoStore {
    fn get_transaction_info(&self, txn_hash: HashValue) -> Result<Option<TransactionInfo>>;
    fn save_transaction_info(&self, txn_info: TransactionInfo) -> Result<()>;
    fn save_transaction_infos(&self, vec_txn_info: Vec<TransactionInfo>) -> Result<()>;
}

pub trait TransactionStore {
    fn get_transaction(&self, txn_hash: HashValue) -> Result<Option<Transaction>>;
    fn save_transaction(&self, txn_info: Transaction) -> Result<()>;
    fn save_transaction_batch(&self, txn_vec: Vec<Transaction>) -> Result<()>;
}

pub struct Storage {
    transaction_info_storage: TransactionInfoStorage,
    transaction_storage: TransactionStorage,
    block_storage: BlockStorage,
    state_node_storage: StateStorage,
    accumulator_storage: AccumulatorStorage,
    block_info_storage: BlockInfoStorage,
    startup_info_storage: Arc<dyn KVStore>,
}

impl Storage {
    pub fn new(instance: StorageInstance) -> Result<Self> {
        Ok(Self {
            transaction_info_storage: TransactionInfoStorage::new(instance.clone()),
            transaction_storage: TransactionStorage::new(instance.clone()),
            block_storage: BlockStorage::new(instance.clone()),
            state_node_storage: StateStorage::new(instance.clone()),
            accumulator_storage: AccumulatorStorage::new(instance.clone()),
            block_info_storage: BlockInfoStorage::new(instance.clone()),
            startup_info_storage: Arc::new(InnerStorage::new(
                instance.clone(),
                STARTUP_INFO_PREFIX_NAME,
            )),
        })
    }
}

impl StateNodeStore for Storage {
    fn get(&self, hash: &HashValue) -> Result<Option<StateNode>> {
        self.state_node_storage.get(hash.clone())
    }

    fn put(&self, key: HashValue, node: StateNode) -> Result<()> {
        self.state_node_storage.put(key, node)
    }

    fn write_nodes(&self, nodes: BTreeMap<HashValue, StateNode>) -> Result<(), Error> {
        self.state_node_storage.write_nodes(nodes)
    }
}

impl BlockStore for Storage {
    fn get_startup_info(&self) -> Result<Option<StartupInfo>> {
        self.startup_info_storage
            .get(STARTUP_INFO_PREFIX_NAME.as_bytes())
            .and_then(|bytes| match bytes {
                Some(bytes) => Ok(Some(bytes.try_into()?)),
                None => Ok(None),
            })
    }

    fn save_startup_info(&self, startup_info: StartupInfo) -> Result<()> {
        self.startup_info_storage.put(
            STARTUP_INFO_PREFIX_NAME.as_bytes().to_vec(),
            startup_info.try_into()?,
        )
    }

    fn get_headers(&self) -> Result<Vec<HashValue>> {
        self.block_storage.get_headers()
    }

    fn save_branch_number(
        &self,
        branch_id: HashValue,
        number: u64,
        block_id: HashValue,
    ) -> Result<(), Error> {
        self.block_storage
            .save_branch_number(branch_id, number, block_id)
    }

    fn get_block(&self, block_id: HashValue) -> Result<Option<Block>> {
        self.block_storage.get(block_id)
    }

    fn get_body(&self, block_id: HashValue) -> Result<Option<BlockBody>> {
        self.block_storage.get_body(block_id)
    }

    fn get_branch_number(
        &self,
        branch_id: HashValue,
        number: u64,
    ) -> Result<Option<HashValue>, Error> {
        self.block_storage.get_branch_number(branch_id, number)
    }

    fn get_number(&self, number: u64) -> Result<Option<HashValue>> {
        self.block_storage.get_number(number)
    }

    fn commit_branch_block(&self, branch_id: HashValue, block: Block) -> Result<()> {
        self.block_storage.commit_branch_block(branch_id, block)
    }

    fn get_branch_hashes(&self, block_id: HashValue) -> Result<Vec<HashValue>> {
        self.block_storage.get_branch_hashes(block_id)
    }

    fn get_latest_block_header(&self) -> Result<Option<BlockHeader>> {
        self.block_storage.get_latest_block_header()
    }

    fn get_latest_block(&self) -> Result<Block> {
        self.block_storage.get_latest_block()
    }

    fn get_block_header_by_hash(&self, block_id: HashValue) -> Result<Option<BlockHeader>> {
        self.block_storage.get_block_header_by_hash(block_id)
    }

    fn get_block_by_hash(&self, block_id: HashValue) -> Result<Option<Block>> {
        self.block_storage.get_block_by_hash(block_id)
    }

    fn get_block_header_by_number(&self, number: u64) -> Result<Option<BlockHeader>> {
        self.block_storage.get_block_header_by_number(number)
    }

    fn get_header_by_branch_number(
        &self,
        branch_id: HashValue,
        number: u64,
    ) -> Result<Option<BlockHeader>, Error> {
        self.block_storage
            .get_header_by_branch_number(branch_id, number)
    }

    fn get_block_by_number(&self, number: u64) -> Result<Option<Block>> {
        self.block_storage.get_block_by_number(number)
    }

    fn get_block_by_branch_number(
        &self,
        branch_id: HashValue,
        number: u64,
    ) -> Result<Option<Block>, Error> {
        self.block_storage
            .get_block_by_branch_number(branch_id, number)
    }

    fn get_common_ancestor(
        &self,
        block_id1: HashValue,
        block_id2: HashValue,
    ) -> Result<Option<HashValue>> {
        self.block_storage.get_common_ancestor(block_id1, block_id2)
    }

    fn get_block_transactions(&self, block_id: HashValue) -> Result<Vec<HashValue>, Error> {
        self.block_storage.get_transactions(block_id)
    }
    fn save_block_transactions(
        &self,
        block_id: HashValue,
        transactions: Vec<HashValue>,
    ) -> Result<()> {
        self.block_storage.put_transactions(block_id, transactions)
    }
}

impl AccumulatorTreeStore for Storage {}
impl AccumulatorReader for Storage {
    ///get node by node hash
    fn get_node(&self, hash: HashValue) -> Result<Option<AccumulatorNode>> {
        self.accumulator_storage.get_node(hash)
    }

    fn multiple_get(&self, _hash_vec: Vec<HashValue>) -> Result<Vec<AccumulatorNode>, Error> {
        unimplemented!()
    }
}

impl AccumulatorWriter for Storage {
    /// save node
    fn save_node(&self, node: AccumulatorNode) -> Result<()> {
        self.accumulator_storage.save_node(node)
    }
    ///delete node
    fn delete_nodes(&self, node_hash_vec: Vec<HashValue>) -> Result<()> {
        self.accumulator_storage.delete_nodes(node_hash_vec)
    }
}

impl BlockInfoStore for Storage {
    fn save_block_info(&self, block_info: BlockInfo) -> Result<(), Error> {
        self.block_info_storage.put(block_info.block_id, block_info)
    }

    fn get_block_info(&self, hash_value: HashValue) -> Result<Option<BlockInfo>, Error> {
        self.block_info_storage.get(hash_value)
    }
}

impl TransactionInfoStore for Storage {
    fn get_transaction_info(&self, txn_hash: HashValue) -> Result<Option<TransactionInfo>> {
        self.transaction_info_storage.get(txn_hash)
    }

    fn save_transaction_info(&self, txn_info: TransactionInfo) -> Result<()> {
        self.transaction_info_storage
            .put(txn_info.transaction_hash(), txn_info)
    }

    fn save_transaction_infos(&self, vec_txn_info: Vec<TransactionInfo>) -> Result<(), Error> {
        self.transaction_info_storage
            .save_transaction_infos(vec_txn_info)
    }
}

impl TransactionStore for Storage {
    fn get_transaction(&self, txn_hash: HashValue) -> Result<Option<Transaction>, Error> {
        self.transaction_storage.get(txn_hash)
    }

    fn save_transaction(&self, txn: Transaction) -> Result<(), Error> {
        self.transaction_storage.put(txn.id(), txn)
    }

    fn save_transaction_batch(&self, txn_vec: Vec<Transaction>) -> Result<(), Error> {
        self.transaction_storage.save_transaction_batch(txn_vec)
    }
}

/// Chain storage define
pub trait Store:
    StateNodeStore
    + BlockStore
    + AccumulatorTreeStore
    + BlockInfoStore
    + TransactionStore
    + TransactionInfoStore
    + IntoSuper<dyn StateNodeStore>
    + IntoSuper<dyn AccumulatorTreeStore>
{
}

pub trait IntoSuper<Super: ?Sized> {
    fn as_super(&self) -> &Super;
    fn as_super_mut(&mut self) -> &mut Super;
    fn into_super(self: Box<Self>) -> Box<Super>;
    fn into_super_arc(self: Arc<Self>) -> Arc<Super>;
}

impl<'a, T: 'a + StateNodeStore> IntoSuper<dyn StateNodeStore + 'a> for T {
    fn as_super(&self) -> &(dyn StateNodeStore + 'a) {
        self
    }
    fn as_super_mut(&mut self) -> &mut (dyn StateNodeStore + 'a) {
        self
    }
    fn into_super(self: Box<Self>) -> Box<dyn StateNodeStore + 'a> {
        self
    }
    fn into_super_arc(self: Arc<Self>) -> Arc<dyn StateNodeStore + 'a> {
        self
    }
}

impl<'a, T: 'a + AccumulatorTreeStore> IntoSuper<dyn AccumulatorTreeStore + 'a> for T {
    fn as_super(&self) -> &(dyn AccumulatorTreeStore + 'a) {
        self
    }
    fn as_super_mut(&mut self) -> &mut (dyn AccumulatorTreeStore + 'a) {
        self
    }
    fn into_super(self: Box<Self>) -> Box<dyn AccumulatorTreeStore + 'a> {
        self
    }
    fn into_super_arc(self: Arc<Self>) -> Arc<dyn AccumulatorTreeStore + 'a> {
        self
    }
}

impl Store for Storage {}

///ensure slice length
fn ensure_slice_len_eq(data: &[u8], len: usize) -> Result<()> {
    ensure!(
        data.len() == len,
        "Unexpected data len {}, expected {}.",
        data.len(),
        len,
    );
    Ok(())
}
