# PulseChain EVM Executor

Custom block executor for PulseChain that implements the PrimordialPulse fork modifications at block 17,233,000.

## Overview

This crate provides the EVM execution layer for PulseChain's pulse-reth implementation. It wraps the standard Ethereum executor and applies PulseChain-specific state modifications at the fork block.

## Architecture

The executor follows the custom executor pattern used in reth, wrapping `EthBlockExecutor` while overriding specific behavior:

```
PulseChainEvmConfig (implements ConfigureEvm)
  └─> PulseChainBlockExecutor (implements BlockExecutor)
        └─> EthBlockExecutor (standard Ethereum execution)
```

### Key Components

1. **PulseChainExecutorBuilder** - Builder for creating the custom EVM configuration
2. **PulseChainEvmConfig** - EVM configuration that produces PulseChain block executors
3. **PulseChainBlockExecutor** - Block executor that applies fork modifications
4. **fork_state module** - State modification functions for the PrimordialPulse fork

## PrimordialPulse Fork Implementation

At block **17,233,000**, the executor applies two critical state modifications:

### 1. Sacrifice Credits (292,217 balance increases)

Applies balance increases to addresses that participated in the sacrifice phase:

```rust
apply_sacrifice_credits(evm, &fork.sacrifice_credits)
```

- Loads existing account state or creates new accounts
- Adds credit amounts to balances
- Uses system calls to properly handle state merging

### 2. Deposit Contract Replacement

Replaces the Ethereum deposit contract with PulseChain's version:

```rust
replace_deposit_contract(
    evm,
    ethereum_deposit_address,
    nil_bytecode,
    pulsechain_deposit_address,
    deposit_contract_data,
)
```

- Self-destructs Ethereum deposit contract (0x00000000219ab540356cbb839cbe05303d7705fa)
- Deploys PulseChain deposit contract (0x3693693693693693693693693693693693693693)
- Initializes 31 storage slots with validator data
- Sets nonce to 0 and balance to 0

## Implementation Details

### Execution Flow

1. Block execution starts in `PulseChainBlockExecutor::apply_pre_execution_changes()`
2. Check if current block == 17,233,000
3. If yes, load fork data from embedded resources
4. Apply sacrifice credits (292,217 addresses)
5. Replace deposit contracts
6. Continue with normal Ethereum block execution

### State Modification Approach

The implementation uses two different strategies:

**Sacrifice Credits**: Uses `transact_system_call()` to:
- Properly load existing account state from the database
- Merge balance changes with existing data
- Handle edge cases (existing contracts, non-zero nonces, etc.)

**Deposit Contracts**: Uses direct `DatabaseCommit::commit()` to:
- Replace contracts entirely (no merging needed)
- Batch both contract changes in a single commit
- Ensure atomic state updates

### Technical Choices

- **HashMap Type**: Uses `alloy_primitives::map::HashMap` (with `DefaultHashBuilder`) to match revm's expected type
- **Bytecode**: Accessed via `revm::bytecode::Bytecode` for proper encoding
- **Storage Slots**: Created with `EvmStorageSlot::new(value, transaction_id)`
- **Account Status**: Uses bitflags (`AccountStatus::Touched | AccountStatus::Created`)

## Usage

This executor is designed to be used as part of a full PulseChain node:

```rust
use reth_pulsechain_evm::PulseChainExecutorBuilder;

// In node builder configuration
builder
    .with_components(
        EthereumNode::components()
            .executor(PulseChainExecutorBuilder::default())
    )
    // ... rest of configuration
```

## Testing

The implementation can be tested by:

1. Running a node with mainnet data up to block 17,233,000
2. Verifying state root after fork block matches expected value
3. Checking that all 292,217 sacrifice credit balances are correct
4. Verifying deposit contract code and storage

## Documentation

- **REVM_STATE_API.md** - Comprehensive guide to revm state manipulation APIs
- **Source code** - Inline documentation and comments throughout

## Dependencies

Key dependencies:
- `reth-ethereum` - Base Ethereum execution
- `reth-pulsechain` - Fork data and types
- `alloy-evm` - EVM interface traits
- `revm` - EVM implementation

## References

- [Reth Custom Executor Example](https://github.com/paradigmxyz/reth/tree/main/examples/custom-beacon-withdrawals)
- [PulseChain Specifications](https://gitlab.com/pulsechaincom)
- [REVM Documentation](https://docs.rs/revm)
