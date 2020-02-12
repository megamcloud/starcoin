// Copyright (c) The Libra Core Contributors
// SPDX-License-Identifier: Apache-2.0

use crate::TransactionExecutor;
use anyhow::{Error, Result};
use config::VMConfig;
use state_store::StateStore;
use types::{
    transaction::{SignedUserTransaction, Transaction, TransactionOutput},
    vm_error::VMStatus,
};

pub struct MockExecutor;

impl TransactionExecutor for MockExecutor {
    fn execute_transaction(
        txn: Transaction,
        state_store: &dyn StateStore,
    ) -> Result<TransactionOutput> {
        unimplemented!()
    }

    fn validate_transaction(
        txn: SignedUserTransaction,
        state_store: &dyn StateStore,
    ) -> Result<VMStatus, Error> {
        unimplemented!()
    }
}