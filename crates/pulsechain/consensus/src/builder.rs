//! Consensus builder for PulseChain.

use crate::PulseChainBeaconConsensus;
use alloc::sync::Arc;
use reth_chainspec::{EthChainSpec, EthereumHardforks};
use reth_ethereum_primitives::EthPrimitives;
use reth_node_api::{FullNodeTypes, NodeTypes};
use reth_node_builder::{components::ConsensusBuilder, BuilderContext};

/// Builder for [`PulseChainBeaconConsensus`].
#[derive(Debug, Default, Clone, Copy)]
#[non_exhaustive]
pub struct PulseChainConsensusBuilder;

impl PulseChainConsensusBuilder {
    /// Creates a new instance of the [`PulseChainConsensusBuilder`].
    pub const fn new() -> Self {
        Self
    }
}

impl<Node> ConsensusBuilder<Node> for PulseChainConsensusBuilder
where
    Node: FullNodeTypes<
        Types: NodeTypes<ChainSpec: EthChainSpec + EthereumHardforks, Primitives = EthPrimitives>,
    >,
{
    type Consensus = Arc<PulseChainBeaconConsensus<<Node::Types as NodeTypes>::ChainSpec>>;

    async fn build_consensus(
        self,
        ctx: &BuilderContext<Node>,
    ) -> eyre::Result<Self::Consensus> {
        Ok(Arc::new(PulseChainBeaconConsensus::new(ctx.chain_spec())))
    }
}
