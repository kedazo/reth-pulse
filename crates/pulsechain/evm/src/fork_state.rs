//! Fork state modification functions
//!
//! This module contains the actual implementation of state modifications
//! for the PrimordialPulse fork.

use alloc::format;
use alloy_primitives::{map::HashMap, Address, Bytes, U256};
use core::fmt::Display;
use reth_ethereum::evm::{
    primitives::{
        execute::{BlockExecutionError, InternalBlockExecutionError},
        Evm,
    },
    revm::{
        state::{Account, AccountInfo, AccountStatus, EvmStorageSlot},
        DatabaseCommit,
    },
};
use revm::Database;
use reth_pulsechain::{DepositContractData, SacrificeCredit};
use revm::{
    bytecode::Bytecode,
    primitives::keccak256,
};

/// Apply sacrifice credits to state
///
/// For each credit, adds the credit amount to the address balance.
/// Creates the account if it doesn't exist.
///
/// This loads existing accounts from the database, adds the credit to their balance,
/// and commits all changes at once.
pub(crate) fn apply_sacrifice_credits<E>(
    evm: &mut E,
    credits: &[SacrificeCredit],
) -> Result<(), BlockExecutionError>
where
    E: Evm,
    E::Error: Display,
    E::DB: Database + DatabaseCommit,
{
    tracing::info!(count = credits.len(), "Applying sacrifice credits");

    let mut changes = HashMap::default();

    // For each credit, load existing account state and add balance
    for credit in credits {
        // Load existing account info from database (or get default if account doesn't exist)
        let existing_info = evm
            .db_mut()
            .basic(credit.address)
            .map_err(|e| {
                BlockExecutionError::Internal(InternalBlockExecutionError::Other(
                    format!("Failed to load account {:?}: {}", credit.address, e).into(),
                ))
            })?
            .unwrap_or_default();

        // Calculate new balance (existing + credit)
        let new_balance = existing_info.balance + credit.credit;

        // Create account change with updated balance
        changes.insert(
            credit.address,
            Account {
                info: AccountInfo {
                    balance: new_balance,
                    nonce: existing_info.nonce,
                    code_hash: existing_info.code_hash,
                    code: existing_info.code.clone(),
                },
                transaction_id: 0,
                storage: HashMap::default(), // Don't modify storage
                status: AccountStatus::Touched,
            },
        );
    }

    // Commit all changes at once
    evm.db_mut().commit(changes);

    Ok(())
}

/// Replace Ethereum deposit contract with PulseChain deposit contract
///
/// Steps:
/// 1. Self-destruct Ethereum deposit contract
/// 2. Set Ethereum deposit contract code to nil contract
/// 3. Clear balance on PulseChain deposit contract
/// 4. Deploy PulseChain deposit contract bytecode
/// 5. Set nonce to 0
/// 6. Initialize 31 storage slots (0x22-0x40)
pub(crate) fn replace_deposit_contract<E>(
    evm: &mut E,
    eth_deposit: Address,
    nil_bytecode: &Bytes,
    pulse_deposit: Address,
    deposit_data: &DepositContractData,
) -> Result<(), BlockExecutionError>
where
    E: Evm,
    E::Error: Display,
    E::DB: Database + DatabaseCommit,
{
    tracing::info!(
        eth_deposit = ?eth_deposit,
        pulse_deposit = ?pulse_deposit,
        "Replacing deposit contract"
    );

    let mut changes = HashMap::default();

    // 1. Load existing Ethereum deposit contract (to properly handle existing state)
    let _eth_existing = evm
        .db_mut()
        .basic(eth_deposit)
        .map_err(|e| {
            BlockExecutionError::Internal(InternalBlockExecutionError::Other(
                format!("Failed to load Ethereum deposit contract {:?}: {}", eth_deposit, e).into(),
            ))
        })?;

    // Replace with nil contract (self-destruct)
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
            transaction_id: 0,
            storage: HashMap::default(), // Clear storage
            status: AccountStatus::Touched | AccountStatus::SelfDestructed,
        },
    );

    // 2. Load existing PulseChain deposit contract (if any)
    let _pulse_existing = evm
        .db_mut()
        .basic(pulse_deposit)
        .map_err(|e| {
            BlockExecutionError::Internal(InternalBlockExecutionError::Other(
                format!("Failed to load PulseChain deposit contract {:?}: {}", pulse_deposit, e).into(),
            ))
        })?;

    // Deploy PulseChain deposit contract
    let pulse_code = Bytecode::new_legacy(deposit_data.bytecode.clone());
    let pulse_hash = keccak256(&deposit_data.bytecode);

    // Initialize storage from deposit data
    let mut pulse_storage = HashMap::default();
    for (slot, value) in &deposit_data.storage {
        pulse_storage.insert(*slot, EvmStorageSlot::new((*value).into(), 0));
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
            transaction_id: 0,
            storage: pulse_storage,
            status: AccountStatus::Touched | AccountStatus::Created,
        },
    );

    // Commit all changes at once
    evm.db_mut().commit(changes);

    Ok(())
}
