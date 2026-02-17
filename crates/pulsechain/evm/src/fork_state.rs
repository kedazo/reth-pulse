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
        states::{AccountStatus, StorageSlot, StorageWithOriginalValues, TransitionAccount},
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
    _nil_bytecode: &Bytes,
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

    // 1. Destroy Ethereum deposit contract (SelfDestruct semantics)
    //
    //    CRITICAL FIX (Attempt #13): Fully destroy the account, don't set nil code.
    //    go-pulse does:
    //      state.SelfDestruct(ethereumDepositContractAddr)  // Marks for deletion
    //      state.SetCode(ethereumDepositContractAddr, nilContractBytes)  // Sets code...
    //    BUT go-ethereum's Finalise() deletes any account with selfDestructed==true,
    //    so the SetCode is effectively a no-op. The account is DELETED entirely.
    //
    //    Verified via go-pulse RPC at the latest block:
    //      eth_getCode("0x00000000219ab540356cbb839cbe05303d7705fa") = "0x" (no code)
    //      eth_getBalance(...) = "0x0"
    //      eth_getTransactionCount(...) = "0x0"
    //
    //    Previous Attempt #12 used DestroyedChanged with nil contract code (63 bytes),
    //    which left the account alive with code. This caused EXTCODESIZE > 0, and
    //    ERC-721 safeTransferFrom calls would try onERC721Received on it, consuming
    //    extra gas (5833 gas mismatch at block 21,188,649).
    //
    //    The correct fix: use Destroyed with info: None to fully delete the account.
    let eth_cache_account = state.load_cache_account(eth_deposit).map_err(|e| {
        BlockExecutionError::Internal(InternalBlockExecutionError::Other(
            format!("Failed to load Ethereum deposit contract: {}", e).into(),
        ))
    })?;

    // Capture previous info before modifying
    let eth_previous_info = eth_cache_account.account.as_ref().map(|a| a.info.clone());
    let eth_previous_status = eth_cache_account.status;

    // Create TransitionAccount with Destroyed status (account fully deleted)
    // storage_was_destroyed: true ensures all existing storage is wiped from the trie
    let eth_transition = TransitionAccount {
        info: None,                              // Account deleted - no info
        status: AccountStatus::Destroyed,        // Fully destroyed (matches go-pulse SelfDestruct)
        previous_info: eth_previous_info.clone(),
        previous_status: eth_previous_status,
        storage: StorageWithOriginalValues::default(), // No new storage
        storage_was_destroyed: true,             // CRITICAL: Clears all existing storage from trie
    };

    // Update the cache account to reflect destruction
    eth_cache_account.account = None;
    eth_cache_account.status = AccountStatus::Destroyed;

    tracing::info!(
        address = ?eth_deposit,
        previous_status = ?eth_previous_status,
        new_status = ?eth_transition.status,
        storage_was_destroyed = eth_transition.storage_was_destroyed,
        "Prepared transition to DESTROY Ethereum deposit contract (account + storage fully deleted)"
    );

    transitions.push((eth_deposit, eth_transition));

    // 2. Deploy PulseChain deposit contract with code and storage
    //
    //    The PulseChain deposit contract (0x369...369) likely doesn't exist before the fork,
    //    so this is creating a new contract. go-pulse does:
    //      - Check balance, burn if non-zero
    //      - SetCode(pulseDepositContractAddr, depositContractBytes)
    //      - SetNonce(pulseDepositContractAddr, 0)
    //      - SetState() for each storage slot (0x22-0x40)
    let pulse_cache_account = state.load_cache_account(pulse_deposit).map_err(|e| {
        BlockExecutionError::Internal(InternalBlockExecutionError::Other(
            format!("Failed to load cache for PulseChain deposit contract: {}", e).into(),
        ))
    })?;

    // Capture previous info
    let pulse_previous_info = pulse_cache_account.account.as_ref().map(|a| a.info.clone());
    let pulse_previous_status = pulse_cache_account.status;

    let pulse_code = Bytecode::new_legacy(deposit_data.bytecode.clone());
    let pulse_hash = keccak256(&deposit_data.bytecode);
    let pulse_info = AccountInfo {
        balance: U256::ZERO,
        nonce: 0, // go-pulse sets nonce=0 for new contract
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

    // Determine the appropriate status based on whether the account exists
    let (new_status, storage_destroyed) = if pulse_previous_info.is_none() {
        // Account doesn't exist - creating new (InMemoryChange status, no storage to destroy)
        (AccountStatus::InMemoryChange, false)
    } else {
        // Account exists - use DestroyedChanged to clear any existing storage then set new
        (AccountStatus::DestroyedChanged, true)
    };

    // Create TransitionAccount manually
    let pulse_transition = TransitionAccount {
        info: Some(pulse_info.clone()),
        status: new_status,
        previous_info: pulse_previous_info,
        previous_status: pulse_previous_status,
        storage: pulse_storage.clone(),
        storage_was_destroyed: storage_destroyed,
    };

    // Update cache account
    let plain_storage = pulse_storage
        .iter()
        .map(|(k, v)| (*k, v.present_value))
        .collect();
    pulse_cache_account.account = Some(revm::database::states::PlainAccount {
        info: pulse_info,
        storage: plain_storage,
    });
    pulse_cache_account.status = new_status;

    tracing::info!(
        address = ?pulse_deposit,
        previous_status = ?pulse_previous_status,
        new_status = ?pulse_transition.status,
        storage_was_destroyed = pulse_transition.storage_was_destroyed,
        storage_slots = deposit_data.storage.len(),
        "Prepared transition to create PulseChain deposit contract with initialized storage"
    );

    transitions.push((pulse_deposit, pulse_transition));

    // 3. CRITICAL STEP: Apply transitions to transition_state This is what increment_balances()
    //    does and why it works!
    state.apply_transition(transitions);

    tracing::info!(
        "Deposit contract transitions applied to transition_state. \
         Bytecode will be extracted when framework calls merge_transitions() after block execution."
    );

    // NOTE: Do NOT call merge_transitions() here!
    // The framework calls merge_transitions() AFTER block execution
    // (engine/tree/src/tree/metrics.rs:131). Calling it here corrupts the revert chain and
    // causes state root mismatches.
    //
    // Attempt #10 called merge_transitions() here which caused:
    // - State corruption
    // - State root mismatch at block 17232990 during unwind
    // - Sync failure
    //
    // The transitions we created contain bytecode in TransitionAccount.info.code.
    // When the framework calls merge_transitions(), has_new_contract() will extract
    // the bytecode into bundle_state.contracts for database persistence.

    Ok(())
}
