//! Fork state modification functions
//!
//! This module contains the actual implementation of state modifications
//! for the PrimordialPulse fork.

use alloc::{format, vec::Vec};
use alloy_primitives::{map::HashMap, Address, Bytes, U256};
use core::fmt::Display;
use reth_ethereum::evm::primitives::{
    execute::{BlockExecutionError, InternalBlockExecutionError},
    Evm,
};
use revm::{
    bytecode::Bytecode,
    database::State,
    primitives::keccak256,
    state::{Account, AccountInfo, AccountStatus, EvmStorageSlot},
    Database, DatabaseCommit,
};
use reth_pulsechain::{DepositContractData, SacrificeCredit};

/// Apply sacrifice credits to state
///
/// For each credit, adds the credit amount to the address balance.
/// Creates the account if it doesn't exist.
///
/// This uses `State::increment_balances()` which properly creates transitions
/// that are tracked in the bundle state and will be persisted to the database.
pub(crate) fn apply_sacrifice_credits<'db, E, DB>(
    evm: &mut E,
    credits: &[SacrificeCredit],
) -> Result<(), BlockExecutionError>
where
    E: Evm<DB = &'db mut State<DB>>,
    E::Error: Display,
    DB: Database + 'db,
{
    tracing::info!(count = credits.len(), "Applying sacrifice credits");

    // Get access to the state database
    let state: &mut State<DB> = evm.db_mut();

    // Convert credits to (Address, u128) pairs for increment_balances
    // Note: We assume all credits fit in u128. If any credit exceeds u128::MAX,
    // this will truncate. In practice, all sacrifice credits fit in u128.
    let balance_increments: Vec<(Address, u128)> = credits
        .iter()
        .map(|credit| {
            // Convert U256 to u128, saturating at u128::MAX if needed
            let amount: u128 = credit.credit.try_into().unwrap_or_else(|_| {
                tracing::warn!(
                    address = ?credit.address,
                    credit = ?credit.credit,
                    "Sacrifice credit exceeds u128::MAX, using saturated value"
                );
                u128::MAX
            });
            (credit.address, amount)
        })
        .collect();

    // Use State::increment_balances which properly creates transitions
    // This ensures changes are tracked in the bundle state
    state.increment_balances(balance_increments).map_err(|e| {
        BlockExecutionError::Internal(InternalBlockExecutionError::Other(
            format!("Failed to apply sacrifice credits: {}", e).into(),
        ))
    })?;

    tracing::info!("Sacrifice credits applied successfully");

    Ok(())
}

/// Replace Ethereum deposit contract with PulseChain deposit contract
///
/// Steps:
/// 1. Self-destruct Ethereum deposit contract and replace with nil contract
/// 2. Deploy PulseChain deposit contract bytecode with initialized storage
///
/// This uses `DatabaseCommit::commit()` which properly creates transitions
/// that are tracked in the bundle state and will be persisted to the database.
pub(crate) fn replace_deposit_contract<'db, E, DB>(
    evm: &mut E,
    eth_deposit: Address,
    nil_bytecode: &Bytes,
    pulse_deposit: Address,
    deposit_data: &DepositContractData,
) -> Result<(), BlockExecutionError>
where
    E: Evm<DB = &'db mut State<DB>>,
    E::Error: Display,
    DB: Database + 'db,
{
    tracing::info!(
        eth_deposit = ?eth_deposit,
        pulse_deposit = ?pulse_deposit,
        "Replacing deposit contract"
    );

    // Get access to the state database
    let state: &mut State<DB> = evm.db_mut();

    // Build changes to commit
    let mut changes: HashMap<Address, Account> = HashMap::default();

    // 1. Replace Ethereum deposit contract with nil contract
    let nil_code = Bytecode::new_legacy(nil_bytecode.clone());
    let nil_hash = keccak256(nil_bytecode);

    changes.insert(
        eth_deposit,
        Account {
            info: AccountInfo {
                balance: U256::ZERO,
                nonce: 0,
                code_hash: nil_hash,
                code: Some(nil_code),
            },
            storage: HashMap::default(), // Clear all storage
            status: AccountStatus::SelfDestructed | AccountStatus::Touched,
            transaction_id: 0,
        },
    );

    // 2. Deploy PulseChain deposit contract
    let pulse_code = Bytecode::new_legacy(deposit_data.bytecode.clone());
    let pulse_hash = keccak256(&deposit_data.bytecode);

    // Initialize storage from deposit data
    // Use EvmStorageSlot with original_value = 0 (slot didn't exist before)
    // and present_value = the value we want to set
    let mut pulse_storage: HashMap<U256, EvmStorageSlot> = HashMap::default();
    for (slot, value) in &deposit_data.storage {
        pulse_storage.insert(
            *slot,
            EvmStorageSlot {
                original_value: U256::ZERO,
                present_value: U256::from_be_bytes(value.0),
                transaction_id: 0,
                is_cold: false,
            },
        );
    }

    changes.insert(
        pulse_deposit,
        Account {
            info: AccountInfo {
                balance: U256::ZERO,
                nonce: 0,
                code_hash: pulse_hash,
                code: Some(pulse_code),
            },
            storage: pulse_storage,
            status: AccountStatus::Created | AccountStatus::Touched,
            transaction_id: 0,
        },
    );

    // Commit changes using DatabaseCommit which properly creates transitions
    state.commit(changes);

    tracing::info!("Deposit contract replacement committed successfully");

    Ok(())
}
