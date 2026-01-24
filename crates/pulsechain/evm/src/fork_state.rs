//! Fork state modification functions
//!
//! This module contains the actual implementation of state modifications
//! for the PrimordialPulse fork.

use alloc::{format, vec::Vec};
use alloy_primitives::{Address, Bytes, U256};
use core::fmt::Display;
use reth_ethereum::evm::primitives::{
    execute::{BlockExecutionError, InternalBlockExecutionError},
    Evm,
};
use reth_pulsechain::{DepositContractData, SacrificeCredit};
use revm::{
    bytecode::Bytecode,
    database::{
        states::{bundle_state::BundleRetention, StorageSlot, StorageWithOriginalValues},
        State,
    },
    primitives::keccak256,
    Database,
};

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
/// 1. Self-destruct Ethereum deposit contract
/// 2. Deploy PulseChain deposit contract bytecode with initialized storage
///
/// This follows the same pattern as `increment_balances()` by:
/// - Loading accounts into cache
/// - Calling methods that return TransitionAccount
/// - Applying transitions to the state
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

    // CRITICAL INSIGHT from increment_balances() implementation:
    // We must not only create transitions, but also APPLY them to transition_state!
    //
    // Previous attempts (1-5) failed because:
    // - Attempts 1-4: Created transitions but never applied them
    // - Attempt 5: insert_account_with_storage() doesn't create transitions at all
    //
    // The correct pattern (from increment_balances):
    // 1. Load cache accounts
    // 2. Call methods that return TransitionAccount
    // 3. Collect transitions in a Vec
    // 4. Apply transitions via state.apply_transition() ← THIS WAS MISSING!

    use revm::state::AccountInfo;

    // Collect all transitions to apply
    let mut transitions = Vec::new();

    // 1. Replace Ethereum deposit contract with nil contract Ethereum deposit EXISTS, so use
    //    change()
    let eth_cache_account = state.load_cache_account(eth_deposit).map_err(|e| {
        BlockExecutionError::Internal(InternalBlockExecutionError::Other(
            format!("Failed to load Ethereum deposit contract: {}", e).into(),
        ))
    })?;

    let nil_code = Bytecode::new_legacy(nil_bytecode.clone());
    let nil_hash = keccak256(&nil_bytecode);
    let nil_info = AccountInfo {
        balance: U256::ZERO,
        nonce: 0,
        code_hash: nil_hash,
        code: Some(nil_code.clone()),
    };

    // Empty storage for selfdestruct
    let eth_storage = StorageWithOriginalValues::default();

    let eth_transition = eth_cache_account.change(nil_info, eth_storage);

    tracing::debug!(
        address = ?eth_deposit,
        eth_transition_info = ?eth_transition.info,
        eth_transition_previous_info = ?eth_transition.previous_info,
        eth_transition_status = ?eth_transition.status,
        "Prepared transition to replace Ethereum deposit contract with nil contract"
    );

    transitions.push((eth_deposit, eth_transition));

    // 2. Deploy PulseChain deposit contract with code and storage CRITICAL: Use change() not
    //    newly_created()!
    //
    //    increment_balance() (which works) uses account_info_change() internally,
    //    which calls on_changed() not on_created(). The status matters for persistence!
    //
    //    Hypothesis: change() + Changed status persists code field correctly,
    //                newly_created() + InMemoryChange status loses code field
    let pulse_cache_account = state.load_cache_account(pulse_deposit).map_err(|e| {
        BlockExecutionError::Internal(InternalBlockExecutionError::Other(
            format!("Failed to load cache for PulseChain deposit contract: {}", e).into(),
        ))
    })?;

    let pulse_code = Bytecode::new_legacy(deposit_data.bytecode.clone());
    let pulse_hash = keccak256(&deposit_data.bytecode);
    let pulse_info = AccountInfo {
        balance: U256::ZERO,
        nonce: 1,
        code_hash: pulse_hash,
        code: Some(pulse_code.clone()),
    };

    // Convert storage to StorageWithOriginalValues format
    let pulse_storage: StorageWithOriginalValues = deposit_data
        .storage
        .iter()
        .map(|(slot, value)| {
            let storage_value = U256::from_be_bytes(value.0);
            (
                *slot,
                StorageSlot {
                    previous_or_original_value: U256::ZERO, // New account, no previous value
                    present_value: storage_value,
                },
            )
        })
        .collect();

    // Use change() instead of newly_created() - matches increment_balance pattern
    let pulse_transition = pulse_cache_account.change(pulse_info, pulse_storage);

    tracing::info!(
        address = ?pulse_deposit,
        pulse_transition_info_code_hash = ?pulse_transition.info.as_ref().map(|i| i.code_hash),
        pulse_transition_info_has_code = pulse_transition.info.as_ref().and_then(|i| i.code.as_ref()).is_some(),
        pulse_transition_previous_info = ?pulse_transition.previous_info,
        pulse_transition_status = ?pulse_transition.status,
        storage_slots = deposit_data.storage.len(),
        "Prepared transition to create PulseChain deposit contract with initialized storage"
    );

    transitions.push((pulse_deposit, pulse_transition));

    // 3. CRITICAL STEP: Apply transitions to transition_state This is what increment_balances()
    //    does and why it works!
    state.apply_transition(transitions);

    tracing::info!("Deposit contract transitions applied to transition_state");

    // 4. CRITICAL: Merge transitions into bundle_state to extract bytecode!
    //
    // WITHOUT THIS CALL: Bytecode stays in transition_state and is never written to database
    // WITH THIS CALL: merge_transitions() calls has_new_contract() on each TransitionAccount
    //                 and extracts bytecode into bundle_state.contracts for database persistence
    //
    // This is the missing step that caused Attempts 6-9 to fail!
    state.merge_transitions(BundleRetention::Reverts);

    tracing::info!(
        "Transitions merged into bundle_state. Bytecode extracted to bundle_state.contracts for database persistence"
    );

    Ok(())
}
