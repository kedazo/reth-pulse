> **⚠️ SUPERSEDED — DO NOT IMPLEMENT FROM THIS DOCUMENT.**
>
> This is a *pre-implementation design doc* written against revm 34. Two of its recipes are
> known-wrong and were paid for in production debugging:
>
> * `replace_deposit_contract` here sets `Touched | SelfDestructed` **with `code: Some(nil_code)`**.
>   That is Attempt #12's bug: it leaves 63 bytes of code alive, so `EXTCODESIZE > 0` and an
>   ERC-721 `safeTransferFrom` to the deposit address burns +5833 gas — a gas mismatch at block
>   21,188,649. The contract must be **fully destroyed** (`info: None`, `storage_was_destroyed: true`).
> * It uses `EvmStorageSlot::new(value)`. That yields `original == present`, and revm filters
>   storage on `is_changed()` — **all 31 deposit slots are silently dropped**. Use
>   `EvmStorageSlot::new_changed(ZERO, value, TransactionId::ZERO)`.
>
> The authoritative sources are `DEPOSIT_CONTRACT_FIX_LOG.md` (in the old repo) and the
> **actual code** in `src/fork_state.rs`, which targets revm 42. See also the port's `CLAUDE.md`.

# REVM State Manipulation API Documentation

This document describes the correct APIs for manipulating state in revm for the PulseChain fork modifications.

## Overview

State modifications in revm are done through the `DatabaseCommit` trait, which is implemented by `State<DB>`. The key method is:

```rust
fn commit(&mut self, changes: HashMap<Address, Account>)
```

## Key Types

### 1. `AccountInfo`

Located in `revm-state` crate. Contains account balance, nonce, code hash, and optionally bytecode.

```rust
pub struct AccountInfo {
    /// Account balance
    pub balance: U256,
    /// Account nonce
    pub nonce: u64,
    /// Hash of the raw bytes in code, or KECCAK_EMPTY
    pub code_hash: B256,
    /// Bytecode data (optional)
    pub code: Option<Bytecode>,
}
```

**Creation methods:**
- `AccountInfo::default()` - Zero balance, zero nonce, empty code
- `AccountInfo::from_balance(balance)` - Set balance, rest default
- `AccountInfo::from_bytecode(bytecode)` - Set code, balance=0, nonce=1
- `AccountInfo::new(balance, nonce, code_hash, code)` - Full control

**Modification methods:**
- `set_balance(&mut self, balance: U256)` - Set balance
- `set_nonce(&mut self, nonce: u64)` - Set nonce
- `set_code(&mut self, code: Bytecode)` - Set code and calculate hash
- `set_code_and_hash(&mut self, code, hash)` - Set code with known hash

### 2. `Account`

Located in `revm-state` crate. Journal entry for tracking changes.

```rust
pub struct Account {
    /// Balance, nonce, and code
    pub info: AccountInfo,
    /// Transaction ID for tracking
    pub transaction_id: usize,
    /// Storage cache
    pub storage: EvmStorage,  // HashMap<U256, EvmStorageSlot>
    /// Account status flags
    pub status: AccountStatus,
}
```

**Important methods:**
- `mark_touch(&mut self)` - Mark account as modified
- `mark_selfdestruct(&mut self)` - Mark for self-destruct
- `mark_created(&mut self)` - Mark as newly created
- `mark_created_locally(&mut self)` - Mark as created in this transaction

### 3. `EvmStorageSlot`

Storage value wrapper:

```rust
pub struct EvmStorageSlot {
    /// Original value from database
    pub original_value: U256,
    /// Present value
    pub present_value: U256,
}

impl EvmStorageSlot {
    pub fn new(value: U256) -> Self {
        Self {
            original_value: value,
            present_value: value,
        }
    }
}
```

## State Modification Patterns

### Pattern 1: Modify Existing Account Balance

```rust
use revm::DatabaseCommit;
use revm_state::{Account, AccountInfo, AccountStatus};
use alloy_primitives::U256;
use std::collections::HashMap;

fn add_balance_to_account<DB>(
    evm: &mut impl Evm<DB = &mut State<DB>>,
    address: Address,
    amount: U256,
) -> Result<(), BlockExecutionError>
where
    DB: Database,
{
    let mut changes = HashMap::new();

    // Load existing account info (or create if doesn't exist)
    let account_info = match evm.db_mut().basic(address)? {
        Some(info) => info,
        None => AccountInfo::default(),  // New account
    };

    // Create Account struct with modified balance
    let account = Account {
        info: AccountInfo {
            balance: account_info.balance + amount,
            nonce: account_info.nonce,
            code_hash: account_info.code_hash,
            code: account_info.code,
        },
        transaction_id: 0,  // Not relevant for pre-block modifications
        storage: HashMap::default(),
        status: AccountStatus::Touched,  // Mark as modified
    };

    changes.insert(address, account);
    evm.db_mut().commit(changes);

    Ok(())
}
```

### Pattern 2: Deploy Contract with Code and Storage

```rust
use revm::primitives::{Bytecode, keccak256};
use revm_state::EvmStorageSlot;

fn deploy_contract<DB>(
    evm: &mut impl Evm<DB = &mut State<DB>>,
    address: Address,
    bytecode: Bytes,
    storage: Vec<(U256, B256)>,
) -> Result<(), BlockExecutionError>
where
    DB: Database,
{
    let mut changes = HashMap::new();

    // Create bytecode object
    let code = Bytecode::new_legacy(bytecode.clone());
    let code_hash = keccak256(&bytecode);

    // Create storage map
    let mut storage_map = HashMap::default();
    for (slot, value) in storage {
        storage_map.insert(slot, EvmStorageSlot::new(value.into()));
    }

    // Create account
    let account = Account {
        info: AccountInfo {
            balance: U256::ZERO,
            nonce: 0,  // or 1 for contracts
            code_hash,
            code: Some(code),
        },
        transaction_id: 0,
        storage: storage_map,
        status: AccountStatus::Touched | AccountStatus::Created,
    };

    changes.insert(address, account);
    evm.db_mut().commit(changes);

    Ok(())
}
```

### Pattern 3: Self-Destruct Contract

```rust
fn selfdestruct_contract<DB>(
    evm: &mut impl Evm<DB = &mut State<DB>>,
    address: Address,
) -> Result<(), BlockExecutionError>
where
    DB: Database,
{
    let mut changes = HashMap::new();

    // Load existing account
    let account_info = evm.db_mut().basic(address)?
        .ok_or_else(|| BlockExecutionError::Internal(
            InternalBlockExecutionError::Other("Account not found".into())
        ))?;

    // Create self-destructed account
    let mut account = Account {
        info: account_info,
        transaction_id: 0,
        storage: HashMap::default(),  // Clear storage
        status: AccountStatus::Touched | AccountStatus::SelfDestructed,
    };

    // Mark as self-destructed (alternative approach)
    account.mark_selfdestruct();
    account.mark_touch();

    changes.insert(address, account);
    evm.db_mut().commit(changes);

    Ok(())
}
```

### Pattern 4: Replace Contract Code

```rust
fn set_contract_code<DB>(
    evm: &mut impl Evm<DB = &mut State<DB>>,
    address: Address,
    new_bytecode: Bytes,
) -> Result<(), BlockExecutionError>
where
    DB: Database,
{
    let mut changes = HashMap::new();

    // Load existing account (or create)
    let account_info = match evm.db_mut().basic(address)? {
        Some(info) => info,
        None => AccountInfo::default(),
    };

    let code = Bytecode::new_legacy(new_bytecode.clone());
    let code_hash = keccak256(&new_bytecode);

    let account = Account {
        info: AccountInfo {
            balance: account_info.balance,
            nonce: account_info.nonce,
            code_hash,
            code: Some(code),
        },
        transaction_id: 0,
        storage: HashMap::default(),  // Preserve existing or clear
        status: AccountStatus::Touched,
    };

    changes.insert(address, account);
    evm.db_mut().commit(changes);

    Ok(())
}
```

## PulseChain Fork Implementation

### Sacrifice Credits Application

```rust
pub fn apply_sacrifice_credits<E>(
    evm: &mut E,
    credits: &[SacrificeCredit],
) -> Result<(), BlockExecutionError>
where
    E: Evm,
    E::DB: DatabaseCommit,
{
    let mut changes = HashMap::new();

    for credit in credits {
        // Load existing account info
        let account_info = match evm.db().basic(credit.address) {
            Ok(Some(info)) => info,
            Ok(None) => AccountInfo::default(),  // New account
            Err(e) => return Err(BlockExecutionError::Internal(
                InternalBlockExecutionError::Other(
                    format!("Failed to load account: {:?}", e).into()
                )
            )),
        };

        // Create account with increased balance
        let account = Account {
            info: AccountInfo {
                balance: account_info.balance + credit.credit,
                nonce: account_info.nonce,
                code_hash: account_info.code_hash,
                code: account_info.code,
            },
            transaction_id: 0,
            storage: HashMap::default(),
            status: AccountStatus::Touched,
        };

        changes.insert(credit.address, account);
    }

    // Commit all changes at once
    evm.db_mut().commit(changes);

    Ok(())
}
```

### Deposit Contract Replacement

```rust
pub fn replace_deposit_contract<E>(
    evm: &mut E,
    eth_deposit: Address,
    nil_bytecode: &Bytes,
    pulse_deposit: Address,
    deposit_data: &DepositContractData,
) -> Result<(), BlockExecutionError>
where
    E: Evm,
    E::DB: DatabaseCommit,
{
    let mut changes = HashMap::new();

    // 1. Self-destruct Ethereum deposit contract
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
            storage: HashMap::default(),  // Clear storage
            status: AccountStatus::Touched | AccountStatus::SelfDestructed,
        },
    );

    // 2. Deploy PulseChain deposit contract
    let pulse_code = Bytecode::new_legacy(deposit_data.bytecode.clone());
    let pulse_hash = keccak256(&deposit_data.bytecode);

    let mut pulse_storage = HashMap::default();
    for (slot, value) in &deposit_data.storage {
        pulse_storage.insert(*slot, EvmStorageSlot::new((*value).into()));
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

    // Commit all changes
    evm.db_mut().commit(changes);

    Ok(())
}
```

## Important Notes

1. **Batch Changes**: Always collect all changes in a HashMap and commit once
2. **Account Status**: Use `AccountStatus::Touched` for all modifications
3. **Transaction ID**: Set to `0` for pre-block modifications (not in transaction context)
4. **Storage**: Use `EvmStorageSlot::new(value)` for storage values
5. **Code Hash**: Calculate with `keccak256()` or use `Bytecode::hash_slow()`
6. **Imports Needed**:
   ```rust
   use revm::DatabaseCommit;
   use revm::primitives::{Bytecode, keccak256, KECCAK_EMPTY};
   use revm_state::{Account, AccountInfo, AccountStatus, EvmStorageSlot};
   use alloy_primitives::{Address, Bytes, U256, B256};
   use std::collections::HashMap;
   ```

## References

- `revm-state` crate: Account, AccountInfo, AccountStatus
- `revm-database-interface` crate: DatabaseCommit trait
- `revm` crate: Bytecode, keccak256, State<DB>
- revm documentation: https://docs.rs/revm/latest/revm/
