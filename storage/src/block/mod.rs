// Copyright (c) The Starcoin Core Contributors
// SPDX-License-Identifier: Apache-2.0

// Copyright (c) The Starcoin Core Contributors
// SPDX-License-Identifier: Apache-2.0
use super::KeyPrefixName;
use crate::storage::{CodecStorage, KeyCodec, Repository, ValueCodec};
use anyhow::{bail, ensure, Error, Result};
use byteorder::{BigEndian, ReadBytesExt};
use crypto::HashValue;
use logger::prelude::*;
use scs::SCSCodec;
use std::io::Write;
use std::mem::size_of;
use std::sync::{Arc, RwLock};
use types::block::{Block, BlockBody, BlockHeader, BlockNumber};

const BLOCK_KEY_NAME: &'static str = "block";
const BLOCK_KEY_PREFIX_NAME: KeyPrefixName = BLOCK_KEY_NAME;
const BLOCK_HEADER_KEY_PREFIX_NAME: KeyPrefixName = "block_header";
const BLOCK_SONS_KEY_PREFIX_NAME: KeyPrefixName = "block_sons";
const BLOCK_BODY_KEY_PREFIX_NAME: KeyPrefixName = "block_body";
const BLOCK_NUM_KEY_PREFIX_NAME: KeyPrefixName = "block_num";
pub struct BlockStore {
    block_store: CodecStorage<HashValue, Block>,
    header_store: CodecStorage<HashValue, BlockHeader>,
    //store parents relationship
    sons_store: RwLock<CodecStorage<HashValue, Vec<HashValue>>>,
    body_store: CodecStorage<HashValue, BlockBody>,
    number_store: CodecStorage<BlockNumber, HashValue>,
}

impl ValueCodec for Block {
    fn encode_value(&self) -> Result<Vec<u8>> {
        self.encode()
    }

    fn decode_value(data: &[u8]) -> Result<Self> {
        Self::decode(data)
    }
}

impl ValueCodec for BlockHeader {
    fn encode_value(&self) -> Result<Vec<u8>> {
        self.encode()
    }

    fn decode_value(data: &[u8]) -> Result<Self> {
        Self::decode(data)
    }
}

impl ValueCodec for BlockBody {
    fn encode_value(&self) -> Result<Vec<u8>> {
        self.encode()
    }

    fn decode_value(data: &[u8]) -> Result<Self> {
        Self::decode(data)
    }
}

impl ValueCodec for Vec<HashValue> {
    fn encode_value(&self) -> Result<Vec<u8>> {
        let mut encoded = vec![];
        for hash in self {
            encoded.write_all(&hash.to_vec());
        }
        Ok(encoded)
    }

    fn decode_value(data: &[u8]) -> Result<Self> {
        let hash_size = size_of::<HashValue>();
        let mut decoded = vec![];
        let mut ends = hash_size;
        let len = data.len();
        let mut begin: usize = 0;
        loop {
            if ends <= len {
                let hash = HashValue::from_slice(&data[begin..ends])?;
                decoded.push(hash);
            } else {
                break;
            }
            begin = ends;
            ends = ends + hash_size;
        }
        Ok(decoded)
    }
}

impl KeyCodec for BlockNumber {
    fn encode_key(&self) -> Result<Vec<u8>> {
        Ok(self.to_be_bytes().to_vec())
    }

    fn decode_key(data: &[u8]) -> Result<Self, Error> {
        Ok((&data[..]).read_u64::<BigEndian>()?)
    }
}

impl BlockStore {
    pub fn new(
        block_store: Arc<dyn Repository>,
        header_store: Arc<dyn Repository>,
        sons_store: Arc<dyn Repository>,
        body_store: Arc<dyn Repository>,
        number_store: Arc<dyn Repository>,
    ) -> Self {
        BlockStore {
            block_store: CodecStorage::new(block_store, BLOCK_KEY_PREFIX_NAME),
            header_store: CodecStorage::new(header_store, BLOCK_HEADER_KEY_PREFIX_NAME),
            sons_store: RwLock::new(CodecStorage::new(sons_store, BLOCK_SONS_KEY_PREFIX_NAME)),
            body_store: CodecStorage::new(body_store, BLOCK_BODY_KEY_PREFIX_NAME),
            number_store: CodecStorage::new(number_store, BLOCK_NUM_KEY_PREFIX_NAME),
        }
    }

    pub fn save(&self, block: Block) -> Result<()> {
        println!(
            "insert block:{:?}, block:{:?}",
            block.header().id(),
            block.header().parent_hash()
        );
        self.block_store.put(block.header().id(), block)
    }

    pub fn save_header(&self, header: BlockHeader) -> Result<()> {
        self.header_store.put(header.id(), header.clone());
        //save sons relationship
        self.put_sons(header.parent_hash(), header.id())
    }

    pub fn get_headers(&self) -> Result<Vec<HashValue>> {
        let mut key_hashes = vec![];
        for hash in self.header_store.keys().unwrap() {
            let hashval = HashValue::from_slice(hash.as_slice()).unwrap();
            println!("header key:{}", hashval.to_hex());
            key_hashes.push(hashval)
        }
        Ok(key_hashes)
    }

    pub fn save_body(&self, block_id: HashValue, body: BlockBody) -> Result<()> {
        self.body_store.put(block_id, body)
    }
    pub fn save_number(&self, number: BlockNumber, block_id: HashValue) -> Result<()> {
        self.number_store.put(number, block_id)
    }

    pub fn get(&self, block_id: HashValue) -> Result<Option<Block>> {
        self.block_store.get(block_id)
    }

    pub fn get_body(&self, block_id: HashValue) -> Result<Option<BlockBody>> {
        self.body_store.get(block_id)
    }

    pub fn get_number(&self, number: u64) -> Result<Option<HashValue>> {
        self.number_store.get(number)
    }

    pub fn commit_block(&self, block: Block) -> Result<()> {
        let (header, body) = block.clone().into_inner();
        //save header
        let block_id = header.id();
        self.save_header(header.clone());
        //save number
        self.save_number(header.number(), block_id);
        //save body
        self.save_body(block_id, body);
        //save block cache
        self.save(block)
    }

    ///返回某个块到分叉块的路径上所有块的hash
    pub fn get_branch_hashes(&self, block_id: HashValue) -> Result<Vec<HashValue>> {
        let mut vev_hash = Vec::new();
        let mut temp_block_id = block_id;
        loop {
            println!("block_id: {}", temp_block_id.to_hex());
            //get header by block_id
            match self.get_block_header_by_hash(temp_block_id)? {
                Some(header) => {
                    if header.id() != block_id {
                        vev_hash.push(header.id());
                    }
                    temp_block_id = header.parent_hash();
                    match self.get_sons(temp_block_id) {
                        Ok(sons) => {
                            if sons.len() > 1 {
                                break;
                            }
                        }
                        Err(err) => bail!("get sons Error: {:?}", err),
                    }
                }
                None => bail!("Error: can not find block {:?}", temp_block_id),
            }
        }
        Ok(vev_hash)
    }
    /// Get common ancestor
    pub fn get_common_ancestor(
        &self,
        block_id1: HashValue,
        block_id2: HashValue,
    ) -> Result<Option<HashValue>> {
        let mut parent_id1 = block_id1;
        let mut parent_id2 = block_id2;
        let mut found = false;
        info!("common ancestor: {:?}, {:?}", block_id1, block_id2);
        match self.get_relationship(block_id1, block_id2) {
            Ok(Some(hash)) => return Ok(Some(hash)),
            _ => {}
        }
        match self.get_relationship(block_id2, block_id1) {
            Ok(Some(hash)) => return Ok(Some(hash)),
            _ => {}
        }

        loop {
            // info!("block_id: {}", parent_id1.to_hex());
            //get header by block_id
            match self.get_block_header_by_hash(parent_id1)? {
                Some(header) => {
                    parent_id1 = header.parent_hash();
                    ensure!(parent_id1 != HashValue::zero(), "invaild block id is zero.");
                    match self.get_sons(parent_id1) {
                        Ok(sons1) => {
                            info!("parent: {:?}, sons1 : {:?}", parent_id1, sons1);
                            if sons1.len() > 1 {
                                // get parent2 from block2
                                loop {
                                    info!("parent2 : {:?}", parent_id2);
                                    ensure!(
                                        parent_id2 != HashValue::zero(),
                                        "invaild block id is zero."
                                    );
                                    if sons1.contains(&parent_id2) {
                                        found = true;
                                        break;
                                    }
                                    match self.get_block_header_by_hash(parent_id2)? {
                                        Some(header2) => {
                                            parent_id2 = header2.parent_hash();
                                        }
                                        None => {
                                            bail!("Error: can not find block2 {:?}", parent_id2)
                                        }
                                    }
                                }
                                if found {
                                    break;
                                }
                            }
                        }
                        Err(err) => bail!("get sons Error: {:?}", err),
                    }
                }
                None => bail!("Error: can not find block {:?}", parent_id1),
            }
        }
        if found {
            Ok(Some(parent_id1))
        } else {
            bail!("not find common ancestor");
        }
    }

    pub fn get_latest_block_header(&self) -> Result<Option<BlockHeader>> {
        let max_number = self.number_store.get_len()?;
        if max_number == 0 {
            return Ok(None);
        }
        self.get_block_header_by_number(max_number - 1)
    }

    pub fn get_latest_block(&self) -> Result<Block> {
        //get storage current len
        let max_number = self.number_store.get_len()?;
        Ok(self.get_block_by_number(max_number - 1)?.unwrap())
    }

    pub fn get_block_header_by_hash(&self, block_id: HashValue) -> Result<Option<BlockHeader>> {
        self.header_store.get(block_id)
    }

    pub fn get_block_by_hash(&self, block_id: HashValue) -> Result<Option<Block>> {
        self.get(block_id)
    }

    pub fn get_block_header_by_number(&self, number: u64) -> Result<Option<BlockHeader>> {
        match self.number_store.get(number).unwrap() {
            Some(block_id) => self.get_block_header_by_hash(block_id),
            None => bail!("can't find block header by number:{}", number),
        }
    }

    pub fn get_block_by_number(&self, number: u64) -> Result<Option<Block>> {
        match self.number_store.get(number)? {
            Some(block_id) => self.block_store.get(block_id),
            None => Ok(None),
        }
    }

    fn get_relationship(
        &self,
        block_id1: HashValue,
        block_id2: HashValue,
    ) -> Result<Option<HashValue>> {
        match self.get_sons(block_id1) {
            Ok(sons) => {
                if sons.contains(&block_id2) {
                    return Ok(Some(block_id1));
                }
            }
            _ => {}
        }
        Ok(None)
    }

    fn get_sons(&self, parent_hash: HashValue) -> Result<Vec<HashValue>> {
        match self.sons_store.read().unwrap().get(parent_hash)? {
            Some(sons) => Ok(sons),
            None => bail!("cant't find sons: {}", parent_hash),
        }
    }

    fn put_sons(&self, parent_hash: HashValue, son_hash: HashValue) -> Result<()> {
        info!("put son:{}, {}", parent_hash, son_hash);
        match self.get_sons(parent_hash) {
            Ok(mut vec_hash) => {
                info!("branch block:{}, {:?}", parent_hash, vec_hash);
                vec_hash.push(son_hash);
                self.sons_store.write().unwrap().put(parent_hash, vec_hash);
            }
            _ => {
                self.sons_store
                    .write()
                    .unwrap()
                    .put(parent_hash, vec![son_hash]);
            }
        }
        Ok(())
    }
}