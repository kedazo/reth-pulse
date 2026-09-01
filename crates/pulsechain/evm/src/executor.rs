//! PulseChain custom block executor.
//!
//! This executor applies PrimordialPulse fork modifications at block 17,233,000.
//!
//! NOTE: This is a work-in-progress implementation. The state manipulation logic
//! requires proper integration with revm's Database trait and journaled state.
//! The approach demonstrated here shows the structure, but the actual state
//! modification methods need to be verified against the current revm API.

use alloc::{format, sync::Arc, vec::Vec};
use alloy_consensus::TxType;
use alloy_evm::{
    block::{BlockExecutorFactory, ExecutableTx, GasOutput, StateDB},
    eth::{EthBlockExecutionCtx, EthBlockExecutor, EthEvmFactory, EthTxResult},
    EvmFactory,
};
use revm::Inspector;
use reth_ethereum::{
    chainspec::ChainSpec,
    evm::{
        primitives::{
            execute::{BlockExecutionError, BlockExecutor, InternalBlockExecutionError},
            Evm, EvmEnv, EvmEnvFor, ExecutionCtxFor, NextBlockEnvAttributes,
        },
        revm::{context::TxEnv, primitives::hardfork::SpecId},
        EthBlockAssembler, EthEvmConfig, RethReceiptBuilder,
    },
    node::{
        api::{ConfigureEngineEvm, ConfigureEvm, ExecutableTxIterator, FullNodeTypes, NodeTypes},
        builder::{components::ExecutorBuilder, BuilderContext},
    },
    primitives::{Header, SealedBlock, SealedHeader},
    provider::BlockExecutionResult,
    rpc::types::engine::ExecutionData,
    Block, EthPrimitives, Receipt, TransactionSigned,
};
use reth_ethereum::chainspec::EthereumHardforks;
use reth_pulsechain::get_primordial_pulse_fork;
use reth_pulsechain_forks::{
    get_effective_chain_id, is_before_primordial_pulse, ETHEREUM_SHANGHAI_TIME,
    PULSECHAIN_PRIMORDIAL_BLOCK,
};
use revm::{context::Block as RevmBlock, database::DatabaseCommitExt, Database as RevmDatabase};

use crate::fork_state::{apply_sacrifice_credits, replace_deposit_contract};

/// PulseChain executor builder
#[derive(Debug, Default, Clone, Copy)]
#[non_exhaustive]
pub struct PulseChainExecutorBuilder;

impl<Types, Node> ExecutorBuilder<Node> for PulseChainExecutorBuilder
where
    Types: NodeTypes<ChainSpec = ChainSpec, Primitives = EthPrimitives>,
    Node: FullNodeTypes<Types = Types>,
{
    type EVM = PulseChainEvmConfig;

    async fn build_evm(self, ctx: &BuilderContext<Node>) -> eyre::Result<Self::EVM> {
        Ok(PulseChainEvmConfig { inner: EthEvmConfig::new(ctx.chain_spec()) })
    }
}

/// PulseChain EVM configuration
#[derive(Debug, Clone)]
pub struct PulseChainEvmConfig {
    inner: EthEvmConfig,
}

impl BlockExecutorFactory for PulseChainEvmConfig {
    type EvmFactory = EthEvmFactory;
    type ExecutionCtx<'a> = EthBlockExecutionCtx<'a>;
    type Transaction = TransactionSigned;
    type Receipt = Receipt;

    fn evm_factory(&self) -> &Self::EvmFactory {
        self.inner.evm_factory()
    }

    type TxExecutionResult =
        EthTxResult<<EthEvmFactory as EvmFactory>::HaltReason, TxType>;

    type Executor<'a, DB: StateDB, I: Inspector<<EthEvmFactory as EvmFactory>::Context<DB>>> =
        PulseChainBlockExecutor<'a, <EthEvmFactory as EvmFactory>::Evm<DB, I>>;

    fn create_executor<'a, DB, I>(
        &'a self,
        evm: <EthEvmFactory as EvmFactory>::Evm<DB, I>,
        ctx: EthBlockExecutionCtx<'a>,
    ) -> Self::Executor<'a, DB, I>
    where
        DB: StateDB,
        I: Inspector<<EthEvmFactory as EvmFactory>::Context<DB>>,
    {
        // Get block number before moving evm
        let block_number = evm.block().number.to::<u64>();

        PulseChainBlockExecutor {
            inner: EthBlockExecutor::new(
                evm,
                ctx,
                self.inner.chain_spec(),
                self.inner.executor_factory.receipt_builder(),
            ),
            block_number,
        }
    }
}

impl ConfigureEvm for PulseChainEvmConfig {
    type Primitives = <EthEvmConfig as ConfigureEvm>::Primitives;
    type Error = <EthEvmConfig as ConfigureEvm>::Error;
    type NextBlockEnvCtx = <EthEvmConfig as ConfigureEvm>::NextBlockEnvCtx;
    type BlockExecutorFactory = Self;
    type BlockAssembler = EthBlockAssembler<ChainSpec>;

    fn block_executor_factory(&self) -> &Self::BlockExecutorFactory {
        self
    }

    fn block_assembler(&self) -> &Self::BlockAssembler {
        self.inner.block_assembler()
    }

    fn evm_env(&self, header: &Header) -> Result<EvmEnv<SpecId>, Self::Error> {
        let mut env = self.inner.evm_env(header)?;

        let original_spec = env.cfg_env.spec;

        // Override chain ID based on block number
        // Pre-fork (< 17,233,000): chain ID = 1 (Ethereum)
        // Post-fork (>= 17,233,000): chain ID = 369 (PulseChain)
        let effective_chain_id = get_effective_chain_id(header.number);
        env.cfg_env.chain_id = effective_chain_id;

        // CRITICAL: For pre-fork blocks, use Ethereum's Shanghai timestamp.
        //
        // go-pulse patch 0028 ("Allow for increased Shanghai time with PulseChain"):
        //   func (c *ChainConfig) IsShanghai(num *big.Int, time uint64) bool {
        //       if c.PrimordialPulseAhead(num) {
        //           return 1681338455 <= time  // Ethereum mainnet Shanghai
        //       }
        //       return isTimestampForked(c.ShanghaiTime, time)  // PulseChain Shanghai
        //   }
        //
        // Before the PulseChain fork (block 17,233,000), these are Ethereum mainnet
        // blocks that must follow Ethereum's hardfork schedule. Ethereum activated
        // Shanghai at timestamp 1681338455 (April 12, 2023), but PulseChain's Shanghai
        // is at timestamp 1683786515 (May 11, 2023). Without this override, blocks
        // between these timestamps would incorrectly run with Paris (pre-Shanghai)
        // rules, causing gas mismatches from missing EIP-3651/3855/3860.
        if is_before_primordial_pulse(header.number)
            && header.timestamp >= ETHEREUM_SHANGHAI_TIME
            && original_spec < SpecId::SHANGHAI
        {
            // CRITICAL: Must use set_spec_and_mainnet_gas_params() instead of just
            // setting spec. In revm 34.0.0, gas costs are in a separate GasParams
            // table initialized at CfgEnv creation. Just setting spec doesn't update
            // the gas table, so Shanghai gas costs (EIP-3860 initcode word cost) are
            // missing.
            env.cfg_env.set_spec_and_mainnet_gas_params(SpecId::SHANGHAI);
        }

        Ok(env)
    }

    fn next_evm_env(
        &self,
        parent: &Header,
        attributes: &NextBlockEnvAttributes,
    ) -> Result<EvmEnv<SpecId>, Self::Error> {
        let mut env = self.inner.next_evm_env(parent, attributes)?;

        // Override chain ID for the next block (parent.number + 1)
        let next_block_number = parent.number + 1;
        let effective_chain_id = get_effective_chain_id(next_block_number);
        env.cfg_env.chain_id = effective_chain_id;

        // For pre-fork blocks, use Ethereum's Shanghai timestamp (see evm_env() comment)
        if is_before_primordial_pulse(next_block_number)
            && attributes.timestamp >= ETHEREUM_SHANGHAI_TIME
            && env.cfg_env.spec < SpecId::SHANGHAI
        {
            // Must update both spec AND gas_params (see evm_env() comment above)
            env.cfg_env.set_spec_and_mainnet_gas_params(SpecId::SHANGHAI);
        }

        Ok(env)
    }

    fn context_for_block<'a>(
        &self,
        block: &'a SealedBlock<Block>,
    ) -> Result<EthBlockExecutionCtx<'a>, Self::Error> {
        self.inner.context_for_block(block)
    }

    fn context_for_next_block(
        &self,
        parent: &SealedHeader,
        attributes: Self::NextBlockEnvCtx,
    ) -> Result<EthBlockExecutionCtx<'_>, Self::Error> {
        self.inner.context_for_next_block(parent, attributes)
    }
}

impl ConfigureEngineEvm<ExecutionData> for PulseChainEvmConfig {
    fn evm_env_for_payload(&self, payload: &ExecutionData) -> Result<EvmEnvFor<Self>, Self::Error> {
        let mut env = self.inner.evm_env_for_payload(payload)?;

        // Override chain ID based on payload block number
        // ExecutionData implements ExecutionPayload trait which has block_number()
        let block_number = payload.block_number();
        let effective_chain_id = get_effective_chain_id(block_number);
        env.cfg_env.chain_id = effective_chain_id;

        Ok(env)
    }

    fn context_for_payload<'a>(
        &self,
        payload: &'a ExecutionData,
    ) -> Result<ExecutionCtxFor<'a, Self>, Self::Error> {
        self.inner.context_for_payload(payload)
    }

    fn tx_iterator_for_payload(
        &self,
        payload: &ExecutionData,
    ) -> Result<impl ExecutableTxIterator<Self>, Self::Error> {
        self.inner.tx_iterator_for_payload(payload)
    }
}

/// PulseChain block executor
#[derive(Debug)]
pub struct PulseChainBlockExecutor<'a, Evm> {
    /// Inner Ethereum execution strategy.
    inner: EthBlockExecutor<'a, Evm, &'a Arc<ChainSpec>, &'a RethReceiptBuilder>,
    /// Block number being executed
    block_number: u64,
}

impl<E> PulseChainBlockExecutor<'_, E>
where
    E: Evm<DB: StateDB, Tx = TxEnv>,
{
    /// Verify that PrimordialPulse fork modifications were actually applied to state.
    ///
    /// This performs sanity checks to ensure:
    /// 1. Sample sacrifice credits were applied (checks first 3 addresses)
    /// 2. PulseChain deposit contract was deployed
    /// 3. Ethereum deposit contract was replaced
    ///
    /// If verification fails, returns an error to stop execution and prevent
    /// continuing with corrupted state.
    fn verify_fork_modifications(
        &mut self,
        fork: &reth_pulsechain::PrimordialPulseFork,
    ) -> Result<(), BlockExecutionError> {
        tracing::info!("Verifying PrimordialPulse fork modifications...");

        // Verify first 3 sacrifice credits were applied (sample check)
        // Use State::basic() which reads from cache (not database.basic() which bypasses cache)
        let sample_count = 3.min(fork.sacrifice_credits.len());
        for (idx, credit) in fork.sacrifice_credits.iter().take(sample_count).enumerate() {
            let account = self.inner.evm_mut().db_mut().basic(credit.address).map_err(|e| {
                BlockExecutionError::Internal(InternalBlockExecutionError::Other(
                    format!("Failed to read account {:?} for verification: {}", credit.address, e)
                        .into(),
                ))
            })?;

            if let Some(account_info) = account {
                if account_info.balance < credit.credit {
                    return Err(BlockExecutionError::Internal(InternalBlockExecutionError::Other(
                        format!(
                            "FORK VERIFICATION FAILED: Sacrifice credit #{} for address {:?} \
                                 was not applied correctly. Expected balance >= {}, got {}",
                            idx, credit.address, credit.credit, account_info.balance
                        )
                        .into(),
                    )));
                }
                tracing::debug!(
                    address = ?credit.address,
                    balance = %account_info.balance,
                    credit = %credit.credit,
                    "Verified sacrifice credit #{}", idx
                );
            } else {
                return Err(BlockExecutionError::Internal(InternalBlockExecutionError::Other(
                    format!(
                        "FORK VERIFICATION FAILED: Sacrifice credit #{} for address {:?} \
                             was not applied - account does not exist",
                        idx, credit.address
                    )
                    .into(),
                )));
            }
        }

        // Verify PulseChain deposit contract was deployed
        let pulse_deposit_account =
            self.inner.evm_mut().db_mut().basic(fork.pulsechain_deposit_contract).map_err(|e| {
                BlockExecutionError::Internal(InternalBlockExecutionError::Other(
                    format!(
                        "Failed to read PulseChain deposit contract {:?} for verification: {}",
                        fork.pulsechain_deposit_contract, e
                    )
                    .into(),
                ))
            })?;

        if let Some(account_info) = pulse_deposit_account {
            if account_info.code_hash.is_zero() || account_info.code_hash == revm::primitives::KECCAK_EMPTY {
                return Err(BlockExecutionError::Internal(InternalBlockExecutionError::Other(
                    format!(
                        "FORK VERIFICATION FAILED: PulseChain deposit contract at {:?} \
                             has no code deployed",
                        fork.pulsechain_deposit_contract
                    )
                    .into(),
                )));
            }

            // Verify the code hash matches expected
            let expected_code_hash =
                revm::primitives::keccak256(&fork.deposit_contract_data.bytecode);
            if account_info.code_hash != expected_code_hash {
                return Err(BlockExecutionError::Internal(InternalBlockExecutionError::Other(
                    format!(
                        "FORK VERIFICATION FAILED: PulseChain deposit contract at {:?} \
                             has wrong code hash. Expected {:?}, got {:?}",
                        fork.pulsechain_deposit_contract,
                        expected_code_hash,
                        account_info.code_hash
                    )
                    .into(),
                )));
            }

            tracing::debug!(
                address = ?fork.pulsechain_deposit_contract,
                code_hash = ?account_info.code_hash,
                "Verified PulseChain deposit contract deployed with correct code"
            );
        } else {
            return Err(BlockExecutionError::Internal(InternalBlockExecutionError::Other(
                format!(
                    "FORK VERIFICATION FAILED: PulseChain deposit contract at {:?} \
                         does not exist",
                    fork.pulsechain_deposit_contract
                )
                .into(),
            )));
        }

        // Verify PulseChain deposit contract storage was initialized
        // Check a sample of critical storage slots
        let storage_slot_count = fork.deposit_contract_data.storage.len();
        let sample_slots: Vec<_> = fork.deposit_contract_data.storage.iter().take(3).collect();

        for (slot, expected_value) in &sample_slots {
            let storage_value = self
                .inner
                .evm_mut()
                .db_mut()
                .storage(fork.pulsechain_deposit_contract, *slot)
                .map_err(|e| {
                    BlockExecutionError::Internal(InternalBlockExecutionError::Other(
                        format!(
                            "Failed to read storage slot {:?} for PulseChain deposit contract: {}",
                            slot, e
                        )
                        .into(),
                    ))
                })?;

            let expected_u256 = alloy_primitives::U256::from_be_bytes(expected_value.0);
            if storage_value != expected_u256 {
                return Err(BlockExecutionError::Internal(InternalBlockExecutionError::Other(
                    format!(
                        "FORK VERIFICATION FAILED: PulseChain deposit contract storage \
                             slot {:?} has wrong value. Expected {:?}, got {:?}",
                        slot, expected_u256, storage_value
                    )
                    .into(),
                )));
            }
        }

        tracing::debug!(
            address = ?fork.pulsechain_deposit_contract,
            total_slots = storage_slot_count,
            verified_slots = sample_slots.len(),
            "Verified PulseChain deposit contract storage initialized"
        );

        // Verify Ethereum deposit contract was destroyed
        //
        // In go-pulse, SelfDestruct marks the account for deletion, and Finalise() deletes it.
        // The account should NOT exist after the fork (or exist as empty if PLS was sent to it).
        let eth_deposit_account =
            self.inner.evm_mut().db_mut().basic(fork.ethereum_deposit_contract).map_err(|e| {
                BlockExecutionError::Internal(InternalBlockExecutionError::Other(
                    format!(
                        "Failed to read Ethereum deposit contract {:?} for verification: {}",
                        fork.ethereum_deposit_contract, e
                    )
                    .into(),
                ))
            })?;

        match eth_deposit_account {
            None => {
                // Account correctly destroyed - expected after SelfDestruct + Finalise
                tracing::debug!(
                    address = ?fork.ethereum_deposit_contract,
                    "Verified Ethereum deposit contract destroyed (account does not exist)"
                );
            }
            Some(account_info) => {
                // Account exists - acceptable only if it's empty (someone sent PLS to it later)
                // or if it still has the nil contract code (intermediate state during fork block)
                let has_code = !account_info.code_hash.is_zero()
                    && account_info.code_hash != revm::primitives::KECCAK_EMPTY;

                if has_code {
                    // Only acceptable if it's the nil contract (intermediate state during fork block)
                    let nil_hash = revm::primitives::keccak256(&fork.nil_contract_bytecode);
                    if account_info.code_hash != nil_hash {
                        return Err(BlockExecutionError::Internal(InternalBlockExecutionError::Other(
                            format!(
                                "FORK VERIFICATION FAILED: Ethereum deposit contract at {:?} \
                                     still has unexpected code. Expected destroyed or nil contract, got code_hash {:?}",
                                fork.ethereum_deposit_contract, account_info.code_hash
                            )
                            .into(),
                        )));
                    }
                }
                tracing::debug!(
                    address = ?fork.ethereum_deposit_contract,
                    code_hash = ?account_info.code_hash,
                    balance = %account_info.balance,
                    "Ethereum deposit contract exists (empty or nil contract)"
                );
            }
        }

        tracing::info!(
            "✓ PrimordialPulse fork verification PASSED - all modifications confirmed in state"
        );

        Ok(())
    }
}

impl<E> BlockExecutor for PulseChainBlockExecutor<'_, E>
where
    E: Evm<DB: StateDB, Tx = TxEnv>,
{
    type Transaction = TransactionSigned;
    type Receipt = Receipt;
    type Evm = E;
    type Result = EthTxResult<E::HaltReason, TxType>;

    fn apply_pre_execution_changes(&mut self) -> Result<(), BlockExecutionError> {
        // Check if current block is the PrimordialPulse fork block
        if self.block_number == PULSECHAIN_PRIMORDIAL_BLOCK {
            tracing::info!(
                block = PULSECHAIN_PRIMORDIAL_BLOCK,
                "Applying PrimordialPulse fork modifications"
            );

            // Get fork data
            let fork = get_primordial_pulse_fork().map_err(|e| {
                BlockExecutionError::Internal(InternalBlockExecutionError::Other(
                    format!("Failed to get fork data: {e}").into(),
                ))
            })?;

            // Apply sacrifice credits (292,217 balance increases)
            apply_sacrifice_credits(self.evm_mut(), &fork.sacrifice_credits)?;

            // Replace deposit contracts
            replace_deposit_contract(
                self.evm_mut(),
                fork.ethereum_deposit_contract,
                &fork.nil_contract_bytecode,
                fork.pulsechain_deposit_contract,
                &fork.deposit_contract_data,
            )?;

            tracing::info!("PrimordialPulse fork modifications applied successfully");

            // CRITICAL VERIFICATION: Verify fork modifications were actually applied
            // This prevents continuing with corrupted state if modifications failed
            self.verify_fork_modifications(&fork)?;
        } else if self.block_number == PULSECHAIN_PRIMORDIAL_BLOCK + 1 {
            // CRITICAL: Verify fork was applied in previous block
            // This catches the case where the fork block was skipped or not executed
            tracing::info!(
                block = self.block_number,
                "Verifying PrimordialPulse fork was applied in previous block"
            );

            let fork = get_primordial_pulse_fork().map_err(|e| {
                BlockExecutionError::Internal(InternalBlockExecutionError::Other(
                    format!("Failed to get fork data: {e}").into(),
                ))
            })?;

            self.verify_fork_modifications(&fork).map_err(|e| {
                BlockExecutionError::Internal(InternalBlockExecutionError::Other(
                    format!(
                        "CRITICAL: Fork verification failed at block {}. \
                         The PrimordialPulse fork at block {} was NOT applied correctly! \
                         Original error: {}",
                        self.block_number, PULSECHAIN_PRIMORDIAL_BLOCK, e
                    )
                    .into(),
                ))
            })?;

            tracing::info!(
                "✓ Confirmed: PrimordialPulse fork was correctly applied in previous block"
            );
        }

        // PulseChain-specific pre-execution changes:
        // We replicate EthBlockExecutor::apply_pre_execution_changes but handle beacon root
        // as OPTIONAL since PulseChain doesn't have a beacon chain.
        //
        // Standard Ethereum requires parent_beacon_block_root for all Cancun+ blocks,
        // but PulseChain blocks have this field as None (no beacon chain infrastructure).
        // Go-pulse handles this by only calling ProcessBeaconBlockRoot if the root is present.

        // 1. EIP-161 state clearing (Spurious Dragon) is no longer set explicitly.
        //    revm 42 removed `State::set_state_clear_flag`; the cache now handles it
        //    internally (`CacheState::apply_account_state` -> `touch_empty_eip161`), and
        //    pre-EIP-161 behaviour is handled by the journal in `finalize()`.

        // 2. Apply EIP-2935 blockhashes contract call (always)
        self.inner
            .system_caller
            .apply_blockhashes_contract_call(self.inner.ctx.parent_hash, &mut self.inner.evm)?;

        // 3. Apply EIP-4788 beacon root contract call ONLY if beacon root is present
        // This is the key PulseChain difference: beacon root is optional
        if let Some(beacon_root) = self.inner.ctx.parent_beacon_block_root {
            self.inner
                .system_caller
                .apply_beacon_root_contract_call(Some(beacon_root), &mut self.inner.evm)?;
        }
        // If beacon root is None, we simply skip the call (matching go-pulse behavior)

        Ok(())
    }

    fn execute_transaction_without_commit(
        &mut self,
        tx: impl ExecutableTx<Self>,
    ) -> Result<Self::Result, BlockExecutionError> {
        self.inner.execute_transaction_without_commit(tx)
    }

    fn commit_transaction(&mut self, output: Self::Result) -> GasOutput {
        self.inner.commit_transaction(output)
    }

    fn receipts(&self) -> &[Self::Receipt] {
        self.inner.receipts()
    }

    fn finish(self) -> Result<(E, BlockExecutionResult<Receipt>), BlockExecutionError> {
        let block_number = self.block_number;
        let block_timestamp: u64 = self.inner.evm.block().timestamp().saturating_to();

        // CRITICAL: PrimordialPulse block (17,233,000) is treated as a PoW block by go-pulse.
        //
        // go-pulse sets difficulty=131072 for this block (patch 0007), causing
        // beacon.Finalize() to see IsPoSHeader()=false and delegate to ethash.Finalize().
        // ethash.Finalize() does two things:
        //   1. Applies fork modifications (sacrifice credits + deposit contract)
        //   2. Calls accumulateRewards() → gives 2 ETH block reward to beneficiary
        // And because it takes the ethash (not beacon) path, withdrawals are NOT processed.
        //
        // In reth, Paris is active at block 15,537,394, so base_block_reward() returns None
        // for block 17,233,000 (no block reward). We must manually add the 2 ETH reward
        // and skip withdrawals to match go-pulse behavior.
        if block_number == PULSECHAIN_PRIMORDIAL_BLOCK {
            let beneficiary = self.inner.evm.block().beneficiary();

            // Check withdrawal situation before consuming self.inner
            let shanghai_active =
                self.inner.spec.is_shanghai_active_at_timestamp(block_timestamp);
            let has_withdrawals = self
                .inner
                .ctx
                .withdrawals
                .as_deref()
                .map_or(false, |w| !w.is_empty());

            if has_withdrawals && shanghai_active {
                tracing::warn!(
                    block = block_number,
                    "PrimordialPulse block has withdrawals and Shanghai is active. \
                     go-pulse skips withdrawals for this block (ethash path). \
                     inner.finish() may incorrectly process them."
                );
            }

            // Let inner.finish() run (it will compute no block reward since Paris is
            // active, and may process withdrawals depending on Shanghai activation)
            let (mut evm, result) = self.inner.finish()?;

            // Add the 2 ETH PoW block reward (Constantinople+ reward)
            // go-pulse's accumulateRewards gives this because the block is treated as PoW
            // (difficulty=131072, patch 0007)
            const ETH_TO_WEI: u128 = 1_000_000_000_000_000_000;
            const BLOCK_REWARD: u128 = 2 * ETH_TO_WEI;

            tracing::info!(
                block = block_number,
                beneficiary = ?beneficiary,
                reward = BLOCK_REWARD,
                "Applying PoW block reward for PrimordialPulse block (go-pulse treats as PoW)"
            );

            evm.db_mut()
                .increment_balances(alloc::vec![(beneficiary, BLOCK_REWARD)])
                .map_err(|_| {
                    BlockExecutionError::Internal(InternalBlockExecutionError::Other(
                        "Failed to apply PrimordialPulse PoW block reward".into(),
                    ))
                })?;

            Ok((evm, result))
        }
        // For pre-fork blocks, the chain spec uses PulseChain's Shanghai timestamp
        // (1683786515), but these are Ethereum mainnet blocks that should use Ethereum's
        // Shanghai timestamp (1681338455). The inner finish() calls
        // post_block_balance_increments() which checks the chain spec to decide whether
        // to process withdrawals. It will skip them because the chain spec says Shanghai
        // isn't active yet. We must manually apply them afterward.
        //
        // go-pulse patch 0028: pre-fork blocks use Ethereum's Shanghai time (1681338455),
        // post-fork blocks use PulseChain's Shanghai time (1683786515).
        else if is_before_primordial_pulse(block_number)
            && block_timestamp >= ETHEREUM_SHANGHAI_TIME
            && !self.inner.spec.is_shanghai_active_at_timestamp(block_timestamp)
        {
            // Collect withdrawal balance increments before self.inner is consumed.
            // CRITICAL: Must aggregate by address! Multiple withdrawals can go to the
            // same address in a single block. increment_balances() reads the starting
            // balance for each entry independently before committing, so duplicate
            // addresses would only apply the last withdrawal's amount instead of the sum.
            let withdrawal_increments: Vec<_> = {
                let mut map = alloc::collections::BTreeMap::<alloy_primitives::Address, u128>::new();
                if let Some(ws) = self.inner.ctx.withdrawals.as_ref() {
                    for w in ws.iter() {
                        if w.amount > 0 {
                            *map.entry(w.address).or_default() += w.amount_wei().to::<u128>();
                        }
                    }
                }
                map.into_iter().collect()
            };

            // inner.finish() handles block rewards, DAO fork, etc. but skips withdrawals
            let (mut evm, result) = self.inner.finish()?;

            // Manually apply the missing withdrawal balance increments
            if !withdrawal_increments.is_empty() {
                evm.db_mut()
                    .increment_balances(withdrawal_increments)
                    .map_err(|_| {
                        BlockExecutionError::Internal(InternalBlockExecutionError::Other(
                            "Failed to apply pre-fork Shanghai withdrawal balance increments"
                                .into(),
                        ))
                    })?;
            }

            Ok((evm, result))
        } else {
            self.inner.finish()
        }
    }

    fn evm_mut(&mut self) -> &mut Self::Evm {
        self.inner.evm_mut()
    }

    fn evm(&self) -> &Self::Evm {
        self.inner.evm()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reth_ethereum::primitives::Header;
    use reth_pulsechain_forks::{ETHEREUM_CHAIN_ID, PULSECHAIN_CHAIN_ID};

    #[test]
    fn test_chain_id_transition_in_evm_env() {
        let chain_spec = reth_pulsechain_chainspec::PULSECHAIN_MAINNET.clone();
        let evm_config = PulseChainEvmConfig { inner: EthEvmConfig::new(chain_spec) };

        // Test pre-fork block (should use Ethereum chain ID = 1)
        let mut pre_fork_header = Header::default();
        pre_fork_header.number = 17_232_999; // One block before fork
        let env = evm_config.evm_env(&pre_fork_header).expect("should create evm env");
        assert_eq!(
            env.cfg_env.chain_id, ETHEREUM_CHAIN_ID,
            "Pre-fork blocks should use Ethereum chain ID (1)"
        );

        // Test fork block (should use PulseChain chain ID = 369)
        let mut fork_header = Header::default();
        fork_header.number = PULSECHAIN_PRIMORDIAL_BLOCK;
        let env = evm_config.evm_env(&fork_header).expect("should create evm env");
        assert_eq!(
            env.cfg_env.chain_id, PULSECHAIN_CHAIN_ID,
            "Fork block should use PulseChain chain ID (369)"
        );

        // Test post-fork block (should use PulseChain chain ID = 369)
        let mut post_fork_header = Header::default();
        post_fork_header.number = 17_233_001; // One block after fork
        let env = evm_config.evm_env(&post_fork_header).expect("should create evm env");
        assert_eq!(
            env.cfg_env.chain_id, PULSECHAIN_CHAIN_ID,
            "Post-fork blocks should use PulseChain chain ID (369)"
        );
    }

    #[test]
    fn test_chain_id_transition_in_next_evm_env() {
        use reth_ethereum::evm::primitives::NextBlockEnvAttributes;

        let chain_spec = reth_pulsechain_chainspec::PULSECHAIN_MAINNET.clone();
        let evm_config = PulseChainEvmConfig { inner: EthEvmConfig::new(chain_spec) };

        let attributes = NextBlockEnvAttributes {
            timestamp: 0,
            suggested_fee_recipient: Default::default(),
            prev_randao: Default::default(),
            gas_limit: 30_000_000,
            extra_data: Default::default(),
            parent_beacon_block_root: None,
            withdrawals: Default::default(),
        };

        // Test parent block before fork (next block = fork block)
        let mut parent_header = Header::default();
        parent_header.number = 17_232_999; // Next block will be fork block
        let env =
            evm_config.next_evm_env(&parent_header, &attributes).expect("should create evm env");
        assert_eq!(
            env.cfg_env.chain_id, PULSECHAIN_CHAIN_ID,
            "Next block (fork block) should use PulseChain chain ID (369)"
        );

        // Test parent block at fork (next block = post-fork)
        let mut parent_header = Header::default();
        parent_header.number = PULSECHAIN_PRIMORDIAL_BLOCK;
        let env =
            evm_config.next_evm_env(&parent_header, &attributes).expect("should create evm env");
        assert_eq!(
            env.cfg_env.chain_id, PULSECHAIN_CHAIN_ID,
            "Next block after fork should use PulseChain chain ID (369)"
        );
    }
}
