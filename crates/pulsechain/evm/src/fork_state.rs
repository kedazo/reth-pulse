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
use reth_pulsechain::{DepositContractData, SacrificeCredit};
use revm::{
    bytecode::Bytecode,
    primitives::{address, keccak256},
};

/// Dummy system address for fork state modifications
pub(crate) const SYSTEM_ADDRESS: Address = address!("0xfffffffffffffffffffffffffffffffffffffffe");

/// Apply sacrifice credits to state
///
/// For each credit, adds the credit amount to the address balance.
/// Creates the account if it doesn't exist.
///
/// This is implemented using system calls to properly handle state merging.
pub(crate) fn apply_sacrifice_credits<E>(
    evm: &mut E,
    credits: &[SacrificeCredit],
) -> Result<(), BlockExecutionError>
where
    E: Evm,
    E::Error: Display,
    E::DB: DatabaseCommit,
{
    tracing::info!(count = credits.len(), "Applying sacrifice credits");

    // For each credit, execute a value transfer from system address
    // This properly handles loading existing account state and merging changes
    for credit in credits {
        let mut state = match evm.transact_system_call(
            SYSTEM_ADDRESS,
            credit.address,
            Bytes::new(), // Empty calldata - just a value transfer
        ) {
            Ok(res) => res.state,
            Err(e) => {
                return Err(BlockExecutionError::Internal(InternalBlockExecutionError::Other(
                    format!("Failed to apply credit to {:?}: {}", credit.address, e).into(),
                )))
            }
        };

        // The system call creates the state change for the transfer
        // Now we need to manually add the balance since this is a mint, not a transfer
        // Remove the system address (it shouldn't lose balance)
        state.remove(&SYSTEM_ADDRESS);

        // Get or create the account entry for the credit recipient
        let account = state.entry(credit.address).or_insert_with(|| Account {
            info: AccountInfo::default(),
            transaction_id: 0,
            storage: HashMap::default(),
            status: AccountStatus::Touched,
        });

        // Add the credit amount to the balance
        account.info.balance += credit.credit;
        account.status |= AccountStatus::Touched;

        evm.db_mut().commit(state);
    }

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
    E::DB: DatabaseCommit,
{
    tracing::info!(
        eth_deposit = ?eth_deposit,
        pulse_deposit = ?pulse_deposit,
        "Replacing deposit contract"
    );

    let mut changes = HashMap::default();

    // 1. Self-destruct Ethereum deposit contract and replace with nil contract
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

    // 2. Deploy PulseChain deposit contract
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
