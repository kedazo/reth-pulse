//! Fork state modification functions
//!
//! This module contains the actual implementation of state modifications
//! for the PrimordialPulse fork.
//!
//! PORTED TO reth v2.5.1 / revm 42 / alloy-evm 0.38:
//! `BlockExecutorFactory::Executor` is now a GAT quantified over *any* `DB: StateDB`,
//! so these helpers can no longer reach into a concrete `State<DB>` to hand-build
//! `TransitionAccount`s and call `apply_transition()`. They now express the same
//! state changes through `DatabaseCommit::commit()`, and revm derives the identical
//! transitions itself via `CacheState::apply_account_state`:
//!   - `mark_selfdestruct()` -> `CacheAccount::selfdestruct()`
//!     -> `TransitionAccount { info: None, status: Destroyed, storage_was_destroyed: true }`
//!     (this is exactly what the previous hand-built "Attempt #13" transition produced)
//!   - `mark_created()` -> `CacheAccount::newly_created()`
//!     -> status `on_created()`: `InMemoryChange` for a fresh account, or
//!        `DestroyedChanged` when it follows a destroy (see the two-phase commit below).
//! NOTE: in `apply_account_state`, selfdestruct is checked BEFORE created, so the two
//! flags must never be set on the same account in the same commit.

use alloc::{format, vec::Vec};
use alloy_evm::block::StateDB;
use alloy_primitives::{Address, Bytes, U256};
use reth_ethereum::evm::primitives::{
    execute::{BlockExecutionError, InternalBlockExecutionError},
    Evm,
};
use reth_pulsechain::{DepositContractData, SacrificeCredit};
use revm::{
    bytecode::Bytecode,
    database::DatabaseCommitExt,
    primitives::{keccak256, AddressMap},
    state::{Account, AccountInfo, EvmStorageSlot, TransactionId},
    Database, DatabaseCommit,
};

fn exec_err(msg: alloc::string::String) -> BlockExecutionError {
    BlockExecutionError::Internal(InternalBlockExecutionError::Other(msg.into()))
}

/// Apply sacrifice credits to state
///
/// For each credit, adds the credit amount to the address balance.
/// Creates the account if it doesn't exist.
///
/// Uses `DatabaseCommitExt::increment_balances()` (blanket-implemented for every
/// `Database + DatabaseCommit`, i.e. every `StateDB`), which creates transitions that are
/// tracked in the bundle state and persisted. Semantically identical to the previous
/// `State::increment_balances()` call - same trait method, just reached generically.
pub(crate) fn apply_sacrifice_credits<E>(
    evm: &mut E,
    credits: &[SacrificeCredit],
) -> Result<(), BlockExecutionError>
where
    E: Evm<DB: StateDB>,
{
    tracing::info!(count = credits.len(), "Applying sacrifice credits");

    // Convert credits to (Address, u128) pairs for increment_balances.
    // Note: We assume all credits fit in u128. If any credit exceeds u128::MAX,
    // this will truncate. In practice, all sacrifice credits fit in u128.
    let balance_increments: Vec<(Address, u128)> = credits
        .iter()
        .map(|credit| {
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

    evm.db_mut()
        .increment_balances(balance_increments)
        .map_err(|e| exec_err(format!("Failed to apply sacrifice credits: {e:?}")))?;

    tracing::info!("Sacrifice credits applied successfully");

    Ok(())
}

/// Replace Ethereum deposit contract with PulseChain deposit contract
///
/// Steps:
/// 1. Fully destroy the Ethereum deposit contract (account + storage).
/// 2. Deploy the PulseChain deposit contract bytecode with initialized storage.
///
/// go-pulse does:
///   state.SelfDestruct(ethereumDepositContractAddr)
///   state.SetCode(ethereumDepositContractAddr, nilContractBytes)
/// but go-ethereum's Finalise() deletes any account with selfDestructed==true, so the
/// SetCode is a no-op and the account is DELETED entirely. Verified against go-pulse RPC:
///   eth_getCode(0x00000000219ab540356cbb839cbe05303d7705fa) == "0x"
///   eth_getBalance(...) == "0x0", eth_getTransactionCount(...) == "0x0"
///
/// Do NOT leave the account alive with the nil contract code: that makes EXTCODESIZE > 0,
/// so ERC-721 safeTransferFrom tries onERC721Received on it and burns extra gas
/// (previously caused a 5833 gas mismatch at block 21,188,649).
pub(crate) fn replace_deposit_contract<E>(
    evm: &mut E,
    eth_deposit: Address,
    _nil_bytecode: &Bytes,
    pulse_deposit: Address,
    deposit_data: &DepositContractData,
) -> Result<(), BlockExecutionError>
where
    E: Evm<DB: StateDB>,
{
    tracing::info!(
        eth_deposit = ?eth_deposit,
        pulse_deposit = ?pulse_deposit,
        "Replacing deposit contract"
    );

    let db = evm.db_mut();

    // Does the PulseChain deposit address already exist? This decides whether the created
    // account transition ends up as `InMemoryChange` (fresh) or `DestroyedChanged` (replacing
    // an existing account, whose storage must be wiped first).
    let pulse_pre_existing = db
        .basic(pulse_deposit)
        .map_err(|e| exec_err(format!("Failed to load PulseChain deposit contract: {e:?}")))?
        .is_some();

    // --- Phase 1: destroy ---------------------------------------------------------------
    // Ethereum deposit contract is always destroyed. If the PulseChain address already holds
    // an account, destroy it in the same commit so its stale storage is wiped from the trie
    // (equivalent to the old hand-built `storage_was_destroyed: true` transition).
    let mut destroy = AddressMap::<Account>::default();

    let eth_info = db
        .basic(eth_deposit)
        .map_err(|e| exec_err(format!("Failed to load Ethereum deposit contract: {e:?}")))?;
    let mut eth_account = match eth_info {
        Some(info) => Account::from(info),
        None => Account::new_not_existing(TransactionId::ZERO),
    };
    eth_account.mark_touch();
    eth_account.mark_selfdestruct();
    destroy.insert(eth_deposit, eth_account);

    if pulse_pre_existing {
        let info = db
            .basic(pulse_deposit)
            .map_err(|e| exec_err(format!("Failed to load PulseChain deposit contract: {e:?}")))?
            .unwrap_or_default();
        let mut acc = Account::from(info);
        acc.mark_touch();
        acc.mark_selfdestruct();
        destroy.insert(pulse_deposit, acc);
    }

    db.commit(destroy);

    tracing::info!(
        address = ?eth_deposit,
        pulse_pre_existing,
        "Destroyed Ethereum deposit contract (account + storage fully deleted)"
    );

    // --- Phase 2: create the PulseChain deposit contract ---------------------------------
    // Separate commit: `apply_account_state` checks selfdestruct BEFORE created, so a create
    // in the same commit as the destroy would be swallowed by the destroy. Committing after
    // the destroy makes `on_created()` yield `DestroyedChanged` when the account pre-existed
    // and `InMemoryChange` when it did not - matching the previous hand-built transitions.
    //
    // go-pulse: burn any balance, SetCode(depositContractBytes), SetNonce(0),
    // SetState() for each storage slot (0x22-0x40).
    let pulse_code = Bytecode::new_legacy(deposit_data.bytecode.clone());
    let pulse_hash = keccak256(&deposit_data.bytecode);
    let pulse_info = AccountInfo {
        balance: U256::ZERO,
        nonce: 0, // go-pulse sets nonce=0 for the new contract
        code_hash: pulse_hash,
        code: Some(pulse_code),
        account_id: None,
    };

    let mut pulse_account = Account::from(pulse_info);
    pulse_account.storage = deposit_data
        .storage
        .iter()
        .map(|(slot, value)| {
            (
                *slot,
                // original == ZERO so the slot is reported as changed and lands in the bundle
                EvmStorageSlot::new_changed(
                    U256::ZERO,
                    U256::from_be_bytes(value.0),
                    TransactionId::ZERO,
                ),
            )
        })
        .collect();
    pulse_account.mark_touch();
    pulse_account.mark_created();

    let mut create = AddressMap::<Account>::default();
    create.insert(pulse_deposit, pulse_account);
    db.commit(create);

    tracing::info!(
        address = ?pulse_deposit,
        storage_slots = deposit_data.storage.len(),
        "Created PulseChain deposit contract with initialized storage"
    );

    // NOTE: Do NOT call merge_transitions() here. The framework calls it AFTER block
    // execution; calling it here corrupts the revert chain and causes state root mismatches
    // (previously: state root mismatch at block 17232990 during unwind).
    // The transitions carry the bytecode in TransitionAccount.info.code; merge_transitions()
    // extracts it into bundle_state.contracts for persistence.

    Ok(())
}
