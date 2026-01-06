//! PulseChain custom block executor.
//!
//! This executor applies PrimordialPulse fork modifications at block 17,233,000.
//!
//! NOTE: This is a work-in-progress implementation. The state manipulation logic
//! requires proper integration with revm's Database trait and journaled state.
//! The approach demonstrated here shows the structure, but the actual state
//! modification methods need to be verified against the current revm API.

use alloc::{boxed::Box, format, sync::Arc};
use alloy_evm::{
    block::{BlockExecutorFactory, BlockExecutorFor, ExecutableTx},
    eth::{EthBlockExecutionCtx, EthBlockExecutor},
    precompiles::PrecompilesMap,
    revm::context::result::ResultAndState,
    EthEvm, EthEvmFactory,
};
use reth_ethereum::{
    chainspec::ChainSpec,
    evm::{
        primitives::{
            execute::{BlockExecutionError, BlockExecutor, InternalBlockExecutionError},
            Database, Evm, EvmEnv, EvmEnvFor, ExecutionCtxFor, InspectorFor,
            NextBlockEnvAttributes, OnStateHook,
        },
        revm::{context::TxEnv, db::State, primitives::hardfork::SpecId},
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
use reth_pulsechain::get_primordial_pulse_fork;
use reth_pulsechain_forks::{get_effective_chain_id, PULSECHAIN_PRIMORDIAL_BLOCK};

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
        let evm_config = PulseChainEvmConfig { inner: EthEvmConfig::new(ctx.chain_spec()) };

        Ok(evm_config)
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

    fn create_executor<'a, DB, I>(
        &'a self,
        evm: EthEvm<&'a mut State<DB>, I, PrecompilesMap>,
        ctx: EthBlockExecutionCtx<'a>,
    ) -> impl BlockExecutorFor<'a, Self, DB, I>
    where
        DB: Database + 'a,
        I: InspectorFor<Self, &'a mut State<DB>> + 'a,
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

        // Override chain ID based on block number
        // Pre-fork (< 17,233,000): chain ID = 1 (Ethereum)
        // Post-fork (>= 17,233,000): chain ID = 369 (PulseChain)
        let effective_chain_id = get_effective_chain_id(header.number);
        env.cfg_env.chain_id = effective_chain_id;

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

impl<'db, DB, E> BlockExecutor for PulseChainBlockExecutor<'_, E>
where
    DB: Database + 'db,
    E: Evm<DB = &'db mut State<DB>, Tx = TxEnv>,
{
    type Transaction = TransactionSigned;
    type Receipt = Receipt;
    type Evm = E;

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
        }

        self.inner.apply_pre_execution_changes()
    }

    fn execute_transaction_without_commit(
        &mut self,
        tx: impl ExecutableTx<Self>,
    ) -> Result<ResultAndState<E::HaltReason>, BlockExecutionError> {
        self.inner.execute_transaction_without_commit(tx)
    }

    fn commit_transaction(
        &mut self,
        output: ResultAndState<E::HaltReason>,
        tx: impl ExecutableTx<Self>,
    ) -> Result<u64, BlockExecutionError> {
        self.inner.commit_transaction(output, tx)
    }

    fn finish(self) -> Result<(E, BlockExecutionResult<Receipt>), BlockExecutionError> {
        self.inner.finish()
    }

    fn set_state_hook(&mut self, hook: Option<Box<dyn OnStateHook>>) {
        self.inner.set_state_hook(hook)
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
