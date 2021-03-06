// Copyright (c) The Starcoin Core Contributors
// SPDX-License-Identifier: Apache-2.0

use crate::{chain_state::StateStore, system_module_names::*};
use crypto::ed25519::Ed25519Signature;
use libra_state_view::StateView;
use libra_types::{
    account_address::AccountAddress as LibraAccountAddress,
    transaction::{
        TransactionOutput as LibraTransactionOutput, TransactionStatus as LibraTransactionStatus,
    },
    vm_error::{sub_status, StatusCode as LibraStatusCode, VMStatus as LibraVMStatus},
    write_set::WriteSet as LibraWriteSet,
};
use logger::prelude::*;
use move_vm_runtime::MoveVM;
use move_vm_state::{
    data_cache::{BlockDataCache, RemoteCache},
    execution_context::{ExecutionContext, SystemExecutionContext, TransactionExecutionContext},
};
use move_vm_types::chain_state::ChainState as LibraChainState;
use move_vm_types::identifier::create_access_path;
use move_vm_types::loaded_data::types::Type;
use move_vm_types::values::Value;
use once_cell::sync::Lazy;
use starcoin_state_api::ChainState;
use std::collections::BTreeMap;
use std::sync::Arc;
use types::{
    account_config,
    block_metadata::BlockMetadata,
    language_storage::{ModuleId, TypeTag},
    transaction::{
        SignatureCheckedTransaction, SignedUserTransaction, Transaction, TransactionArgument,
        TransactionOutput, TransactionPayload, TransactionStatus, MAX_TRANSACTION_SIZE_IN_BYTES,
    },
    vm_error::{StatusCode, VMStatus},
};
use vm::errors::convert_prologue_runtime_error;
use vm::{
    errors::VMResult,
    gas_schedule::{
        self, AbstractMemorySize, CostTable, GasAlgebra, GasCarrier, GasUnits, GAS_SCHEDULE_NAME,
    },
    transaction_metadata::TransactionMetadata,
};

pub static KEEP_STATUS: Lazy<TransactionStatus> =
    Lazy::new(|| TransactionStatus::Keep(VMStatus::new(StatusCode::EXECUTED)));

// We use 10 as the assertion error code for insufficient balance within the Libra coin contract.
pub static DISCARD_STATUS: Lazy<TransactionStatus> = Lazy::new(|| {
    TransactionStatus::Discard(
        VMStatus::new(StatusCode::ABORTED).with_sub_status(StatusCode::REJECTED_WRITE_SET.into()),
    )
});

// The value should be tuned carefully
pub static MAXIMUM_NUMBER_OF_GAS_UNITS: Lazy<GasUnits<GasCarrier>> =
    Lazy::new(|| GasUnits::new(100_000_000));

#[derive(Clone)]
/// Wrapper of MoveVM
pub struct StarcoinVM {
    move_vm: Arc<MoveVM>,
    gas_schedule: Option<CostTable>,
}

impl StarcoinVM {
    pub fn new() -> Self {
        let inner = MoveVM::new();
        Self {
            move_vm: Arc::new(inner),
            gas_schedule: None,
        }
    }

    fn load_gas_schedule(&mut self, data_cache: &dyn RemoteCache) {
        info!("load gas schedule");
        self.gas_schedule = self.fetch_gas_schedule(data_cache).ok();
    }

    fn fetch_gas_schedule(&mut self, data_cache: &dyn RemoteCache) -> VMResult<CostTable> {
        let address = account_config::association_address();
        let mut ctx = SystemExecutionContext::new(data_cache, GasUnits::new(0));
        let gas_struct_ty = self
            .move_vm
            .resolve_struct_def_by_name(&GAS_SCHEDULE_MODULE, &GAS_SCHEDULE_NAME, &mut ctx, &[])
            .map_err(|_| {
                LibraVMStatus::new(LibraStatusCode::GAS_SCHEDULE_ERROR)
                    .with_sub_status(sub_status::GSE_UNABLE_TO_LOAD_MODULE)
            })?;

        let access_path = create_access_path(address.into(), gas_struct_ty.into_struct_tag()?);

        let data_blob = data_cache
            .get(&access_path)
            .map_err(|_| {
                LibraVMStatus::new(LibraStatusCode::GAS_SCHEDULE_ERROR)
                    .with_sub_status(sub_status::GSE_UNABLE_TO_LOAD_RESOURCE)
            })?
            .ok_or_else(|| {
                LibraVMStatus::new(LibraStatusCode::GAS_SCHEDULE_ERROR)
                    .with_sub_status(sub_status::GSE_UNABLE_TO_LOAD_RESOURCE)
            })?;
        let table: CostTable = scs::from_bytes(&data_blob).map_err(|_| {
            LibraVMStatus::new(LibraStatusCode::GAS_SCHEDULE_ERROR)
                .with_sub_status(sub_status::GSE_UNABLE_TO_DESERIALIZE)
        })?;
        Ok(table)
    }

    fn get_gas_schedule(&self) -> Result<&CostTable, VMStatus> {
        self.gas_schedule
            .as_ref()
            .ok_or_else(|| VMStatus::new(StatusCode::VM_STARTUP_FAILURE))
    }

    fn check_gas(&self, txn: &SignedUserTransaction) -> Result<(), VMStatus> {
        // Do not check gas limit for StateSet transaction.
        if let TransactionPayload::StateSet(_) = txn.payload() {
            return Ok(());
        }

        let raw_bytes_len = AbstractMemorySize::new(txn.raw_txn_bytes_len() as GasCarrier);
        // The transaction is too large.
        if txn.raw_txn_bytes_len() > MAX_TRANSACTION_SIZE_IN_BYTES {
            let error_str = format!(
                "max size: {}, txn size: {}",
                MAX_TRANSACTION_SIZE_IN_BYTES,
                raw_bytes_len.get()
            );
            warn!(
                "[VM] Transaction size too big {} (max {})",
                raw_bytes_len.get(),
                MAX_TRANSACTION_SIZE_IN_BYTES
            );
            return Err(
                VMStatus::new(StatusCode::EXCEEDED_MAX_TRANSACTION_SIZE).with_message(error_str)
            );
        }

        // The submitted max gas units that the transaction can consume is greater than the
        // maximum number of gas units bound
        if txn.max_gas_amount() > MAXIMUM_NUMBER_OF_GAS_UNITS.get() {
            let error_str = format!(
                "max gas units: {}, gas units submitted: {}",
                MAXIMUM_NUMBER_OF_GAS_UNITS.get(),
                txn.max_gas_amount()
            );
            warn!(
                "[VM] Gas unit error; max {}, submitted {}",
                MAXIMUM_NUMBER_OF_GAS_UNITS.get(),
                txn.max_gas_amount()
            );
            return Err(
                VMStatus::new(StatusCode::MAX_GAS_UNITS_EXCEEDS_MAX_GAS_UNITS_BOUND)
                    .with_message(error_str),
            );
        }

        // The submitted transactions max gas units needs to be at least enough to cover the
        // intrinsic cost of the transaction as calculated against the size of the
        // underlying `RawTransaction`
        let min_txn_fee = gas_schedule::calculate_intrinsic_gas(raw_bytes_len);
        if txn.max_gas_amount() < min_txn_fee.get() {
            let error_str = format!(
                "min gas required for txn: {}, gas submitted: {}",
                min_txn_fee.get(),
                txn.max_gas_amount()
            );
            warn!(
                "[VM] Gas unit error; min {}, submitted {}",
                min_txn_fee.get(),
                txn.max_gas_amount()
            );
            return Err(
                VMStatus::new(StatusCode::MAX_GAS_UNITS_BELOW_MIN_TRANSACTION_GAS_UNITS)
                    .with_message(error_str),
            );
        }

        // The submitted gas price is less than the minimum gas unit price set by the VM.
        // NB: MIN_PRICE_PER_GAS_UNIT may equal zero, but need not in the future. Hence why
        // we turn off the clippy warning.
        #[allow(clippy::absurd_extreme_comparisons)]
        let below_min_bound = txn.gas_unit_price() < gas_schedule::MIN_PRICE_PER_GAS_UNIT.get();
        if below_min_bound {
            let error_str = format!(
                "gas unit min price: {}, submitted price: {}",
                gas_schedule::MIN_PRICE_PER_GAS_UNIT.get(),
                txn.gas_unit_price()
            );
            warn!(
                "[VM] Gas unit error; min {}, submitted {}",
                gas_schedule::MIN_PRICE_PER_GAS_UNIT.get(),
                txn.gas_unit_price()
            );
            return Err(
                VMStatus::new(StatusCode::GAS_UNIT_PRICE_BELOW_MIN_BOUND).with_message(error_str)
            );
        }

        // The submitted gas price is greater than the maximum gas unit price set by the VM.
        if txn.gas_unit_price() > gas_schedule::MAX_PRICE_PER_GAS_UNIT.get() {
            let error_str = format!(
                "gas unit max price: {}, submitted price: {}",
                gas_schedule::MAX_PRICE_PER_GAS_UNIT.get(),
                txn.gas_unit_price()
            );
            warn!(
                "[VM] Gas unit error; min {}, submitted {}",
                gas_schedule::MAX_PRICE_PER_GAS_UNIT.get(),
                txn.gas_unit_price()
            );
            return Err(
                VMStatus::new(StatusCode::GAS_UNIT_PRICE_ABOVE_MAX_BOUND).with_message(error_str)
            );
        }
        Ok(())
    }

    fn resolve_type_argument(
        &self,
        ctx: &mut SystemExecutionContext,
        tag: &TypeTag,
    ) -> VMResult<Type> {
        Ok(match tag {
            TypeTag::U8 => Type::U8,
            TypeTag::U64 => Type::U64,
            TypeTag::U128 => Type::U128,
            TypeTag::Bool => Type::Bool,
            TypeTag::Address => Type::Address,
            TypeTag::Vector(tag) => Type::Vector(Box::new(self.resolve_type_argument(ctx, tag)?)),
            TypeTag::Struct(struct_tag) => {
                let module_id = ModuleId::new(struct_tag.address.into(), struct_tag.module.clone());
                let ty_args = struct_tag
                    .type_params
                    .iter()
                    .map(|tag| self.resolve_type_argument(ctx, tag))
                    .collect::<VMResult<Vec<_>>>()?;
                Type::Struct(Box::new(self.move_vm.resolve_struct_def_by_name(
                    &module_id.into(),
                    &struct_tag.name,
                    ctx,
                    &ty_args,
                )?))
            }
        })
    }

    fn verify_transaction_impl(
        &mut self,
        transaction: &SignatureCheckedTransaction,
        _state_view: &dyn StateView,
        remote_cache: &dyn RemoteCache,
        txn_data: &TransactionMetadata,
    ) -> Result<VerifiedTranscationPayload, VMStatus> {
        info!("very transaction");
        let mut ctx = SystemExecutionContext::new(remote_cache, GasUnits::new(0));
        self.check_gas(transaction)?;
        self.load_gas_schedule(remote_cache);
        let gas_schedule = self.get_gas_schedule()?;
        match transaction.payload() {
            TransactionPayload::Script(script) => {
                let result = self.run_prologue(gas_schedule, &mut ctx, &txn_data);
                let ty_args = script
                    .ty_args()
                    .iter()
                    .map(|tag| self.resolve_type_argument(&mut ctx, tag))
                    .collect::<VMResult<Vec<_>>>()?;
                // ToDo: fix me
                match result {
                    Ok(_) => Ok(VerifiedTranscationPayload::Script(
                        script.code().to_vec(),
                        ty_args,
                        script.args().to_vec(),
                    )),
                    Err(e) => return Err(e.into()),
                }
            }
            TransactionPayload::Module(module) => {
                let result = self.run_prologue(gas_schedule, &mut ctx, &txn_data);
                match result {
                    Ok(_) => Ok(VerifiedTranscationPayload::Module(module.code().to_vec())),
                    Err(e) => return Err(e.into()),
                }
            }
            _ => Err(VMStatus::new(StatusCode::UNREACHABLE)),
        }
    }

    pub fn verify_transaction(
        &mut self,
        chain_state: &dyn ChainState,
        txn: SignedUserTransaction,
    ) -> Option<VMStatus> {
        let state_store = StateStore::new(chain_state);
        let data_cache = BlockDataCache::new(&state_store);
        let libra_txn = txn.clone().into();
        let txn_data = TransactionMetadata::new(&libra_txn);
        let signature_verified_txn = match txn.check_signature() {
            Ok(t) => t,
            Err(_) => return Some(VMStatus::new(StatusCode::INVALID_SIGNATURE)),
        };
        match self.verify_transaction_impl(
            &signature_verified_txn,
            &state_store,
            &data_cache,
            &txn_data,
        ) {
            Ok(_) => None,
            Err(err) => {
                if err.major_status == StatusCode::SEQUENCE_NUMBER_TOO_NEW {
                    None
                } else {
                    Some(err)
                }
            }
        }
    }
    fn execute_verified_payload(
        &mut self,
        remote_cache: &mut BlockDataCache<'_>,
        txn_data: &TransactionMetadata,
        payload: VerifiedTranscationPayload,
    ) -> LibraTransactionOutput {
        let mut ctx = TransactionExecutionContext::new(txn_data.max_gas_amount(), remote_cache);
        let mut failed_gas_left = GasUnits::new(0);
        let output = match payload {
            VerifiedTranscationPayload::Module(m) => {
                self.move_vm.publish_module(m, &mut ctx, txn_data)
            }
            VerifiedTranscationPayload::Script(s, ty_args, args) => {
                ////////
                let gas_schedule = match self.get_gas_schedule() {
                    Ok(s) => s,
                    Err(e) => return discard_libra_error_output(e.into()),
                };
                self.move_vm.execute_script(
                    s,
                    gas_schedule,
                    &mut ctx,
                    txn_data,
                    ty_args,
                    convert_txn_args(args),
                )
            }
        }
        .map_err(|err| {
            failed_gas_left = ctx.remaining_gas();
            err
        })
        .and_then(|_| {
            failed_gas_left = ctx.remaining_gas();
            let mut gas_free_ctx = SystemExecutionContext::from(ctx);
            self.run_epilogue(&mut gas_free_ctx, txn_data).ok();
            get_transaction_output(
                &mut gas_free_ctx,
                txn_data,
                LibraVMStatus::new(LibraStatusCode::EXECUTED),
            )
        })
        .unwrap_or_else(|err| {
            let mut gas_free_ctx = SystemExecutionContext::new(remote_cache, failed_gas_left);
            self.run_epilogue(&mut gas_free_ctx, txn_data).ok();
            failed_transaction_output(&mut gas_free_ctx, txn_data, err)
        });
        info!("{:?}", output);
        output
    }

    fn run_prologue<T: LibraChainState>(
        &self,
        gas_schedule: &CostTable,
        chain_state: &mut T,
        txn_data: &TransactionMetadata,
    ) -> VMResult<()> {
        let txn_sequence_number = txn_data.sequence_number();
        let txn_public_key = txn_data.authentication_key_preimage().to_vec();
        let txn_gas_price = txn_data.gas_unit_price().get();
        let txn_max_gas_units = txn_data.max_gas_amount().get();
        let txn_expiration_time = txn_data.expiration_time();
        self.move_vm
            .execute_function(
                &ACCOUNT_MODULE,
                &PROLOGUE_NAME,
                gas_schedule,
                chain_state,
                &txn_data,
                vec![],
                vec![
                    Value::u64(txn_sequence_number),
                    Value::vector_u8(txn_public_key),
                    Value::u64(txn_gas_price),
                    Value::u64(txn_max_gas_units),
                    Value::u64(txn_expiration_time),
                ],
            )
            .map_err(|err| convert_prologue_runtime_error(&err, &txn_data.sender))
    }

    fn run_epilogue<T: LibraChainState>(
        &self,
        chain_state: &mut T,
        txn_data: &TransactionMetadata,
    ) -> VMResult<()> {
        let txn_sequence_number = txn_data.sequence_number();
        let txn_gas_price = txn_data.gas_unit_price().get();
        let txn_max_gas_units = txn_data.max_gas_amount().get();
        let gas_remaining = chain_state.remaining_gas().get();
        let gas_schedule = match self.get_gas_schedule() {
            Ok(s) => s,
            Err(e) => return Err(e.into()),
        };
        self.move_vm.execute_function(
            &ACCOUNT_MODULE,
            &EPILOGUE_NAME,
            gas_schedule,
            chain_state,
            &txn_data,
            vec![],
            vec![
                Value::u64(txn_sequence_number),
                Value::u64(txn_gas_price),
                Value::u64(txn_max_gas_units),
                Value::u64(gas_remaining),
            ],
        )
    }

    fn process_block_metadata(
        &self,
        remote_cache: &mut BlockDataCache<'_>,
        block_metadata: BlockMetadata,
    ) -> VMResult<LibraTransactionOutput> {
        let mut txn_data = TransactionMetadata::default();
        txn_data.sender = account_config::mint_address().into();
        txn_data.max_gas_amount = GasUnits::new(std::u64::MAX);

        let mut interpreter_context =
            TransactionExecutionContext::new(txn_data.max_gas_amount(), remote_cache);
        let gas_schedule = CostTable::zero();

        if let Ok((id, timestamp, author, auth)) = block_metadata.into_inner() {
            let previous_vote: BTreeMap<LibraAccountAddress, Ed25519Signature> = BTreeMap::new();
            let vote_maps = scs::to_bytes(&previous_vote).unwrap();
            let round = 0u64;
            let args = vec![
                Value::u64(round),
                Value::u64(timestamp),
                Value::vector_u8(id),
                Value::vector_u8(vote_maps),
                Value::address(author.into()),
                match auth {
                    Some(prefix) => Value::vector_u8(prefix),
                    None => Value::vector_u8(Vec::new()),
                },
            ];

            self.move_vm.execute_function(
                &LIBRA_BLOCK_MODULE,
                &BLOCK_PROLOGUE,
                &gas_schedule,
                &mut interpreter_context,
                &txn_data,
                vec![],
                args,
            )?
        } else {
            return Err(LibraVMStatus::new(LibraStatusCode::MALFORMED));
        };
        get_transaction_output(
            &mut interpreter_context,
            &txn_data,
            LibraVMStatus::new(LibraStatusCode::EXECUTED),
        )
        .map(|output| {
            remote_cache.push_write_set(output.write_set());
            output
        })
    }

    pub fn execute_transaction(
        &mut self,
        chain_state: &dyn ChainState,
        txn: Transaction,
    ) -> TransactionOutput {
        let mut state_store = StateStore::new(chain_state);
        let mut data_cache = BlockDataCache::new(&state_store);
        self.load_gas_schedule(&data_cache);
        match txn {
            Transaction::UserTransaction(txn) => {
                let libra_txn = txn.clone().into();
                let txn_data = TransactionMetadata::new(&libra_txn);

                // check signature
                let signature_checked_txn = match txn.check_signature() {
                    Ok(t) => Ok(t),
                    Err(_) => Err(VMStatus::new(StatusCode::INVALID_SIGNATURE)),
                };

                let output = match signature_checked_txn {
                    Ok(txn) => {
                        let verified_payload = self.verify_transaction_impl(
                            &txn,
                            &state_store,
                            &data_cache,
                            &txn_data,
                        );
                        let result = match verified_payload {
                            Ok(payload) => {
                                self.execute_verified_payload(&mut data_cache, &txn_data, payload)
                            }
                            Err(e) => discard_libra_error_output(e.into()),
                        };

                        if let LibraTransactionStatus::Keep(_) = result.status() {
                            state_store.add_write_set(result.write_set())
                        };
                        TransactionOutput::from(result)
                    }
                    Err(e) => discard_error_output(e),
                };
                output
            }
            Transaction::BlockMetadata(block_metadata) => {
                let result = self
                    .process_block_metadata(&mut data_cache, block_metadata)
                    .unwrap_or_else(discard_libra_error_output);
                if let LibraTransactionStatus::Keep(_) = result.status() {
                    state_store.add_write_set(result.write_set())
                };
                TransactionOutput::from(result)
            }
            Transaction::StateSet(state_set) => {
                let result_status = match chain_state.apply(state_set) {
                    Ok(_) => KEEP_STATUS.clone(),
                    Err(_) => DISCARD_STATUS.clone(),
                };
                TransactionOutput::new(vec![], 0, result_status)
            }
        }
    }
}

pub(crate) fn discard_error_output(err: VMStatus) -> TransactionOutput {
    info!("discard error output: {:?}", err);
    // Since this transaction will be discarded, no writeset will be included.
    TransactionOutput::new(vec![], 0, TransactionStatus::Discard(err))
}

pub(crate) fn discard_libra_error_output(err: LibraVMStatus) -> LibraTransactionOutput {
    info!("discard error output: {:?}", err);
    // Since this transaction will be discarded, no writeset will be included.
    LibraTransactionOutput::new(
        LibraWriteSet::default(),
        vec![],
        0,
        LibraTransactionStatus::Discard(err),
    )
}

/// Convert the transaction arguments into move values.
fn convert_txn_args(args: Vec<TransactionArgument>) -> Vec<Value> {
    args.into_iter()
        .map(|arg| match arg {
            TransactionArgument::U64(i) => Value::u64(i),
            TransactionArgument::Address(a) => Value::address(a.into()),
            TransactionArgument::Bool(b) => Value::bool(b),
            TransactionArgument::U8Vector(v) => Value::vector_u8(v),
        })
        .collect()
}

fn get_transaction_output(
    ctx: &mut (impl LibraChainState + ExecutionContext),
    txn_data: &TransactionMetadata,
    status: LibraVMStatus,
) -> VMResult<LibraTransactionOutput> {
    let gas_used: u64 = txn_data
        .max_gas_amount()
        .sub(ctx.remaining_gas())
        .mul(txn_data.gas_unit_price())
        .get();
    let write_set = ctx.make_write_set()?;
    Ok(LibraTransactionOutput::new(
        write_set,
        ctx.events().to_vec(),
        gas_used,
        LibraTransactionStatus::Keep(status),
    ))
}

pub fn failed_transaction_output(
    ctx: &mut (impl LibraChainState + ExecutionContext),
    txn_data: &TransactionMetadata,
    error_code: LibraVMStatus,
) -> LibraTransactionOutput {
    match LibraTransactionStatus::from(error_code) {
        LibraTransactionStatus::Keep(status) => {
            get_transaction_output(ctx, txn_data, status).unwrap_or_else(discard_libra_error_output)
        }
        LibraTransactionStatus::Discard(status) => discard_libra_error_output(status),
        _ => unreachable!(),
    }
}

pub enum VerifiedTranscationPayload {
    Script(Vec<u8>, Vec<Type>, Vec<TransactionArgument>),
    Module(Vec<u8>),
}
