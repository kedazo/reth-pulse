//! PulseChain node type configuration.

use reth_chainspec::ChainSpec;
use reth_engine_local::LocalPayloadAttributesBuilder;
use reth_ethereum::node::{
    EthereumAddOns, EthereumEngineValidatorBuilder, EthereumEthApiBuilder, EthereumNetworkBuilder,
    EthereumPayloadBuilder, EthereumPoolBuilder,
};
use reth_ethereum_engine_primitives::{EthBuiltPayload, EthPayloadAttributes};
use reth_ethereum_primitives::EthPrimitives;
use reth_node_api::{FullNodeComponents, PayloadAttributesBuilder};
use reth_node_builder::{
    components::{BasicPayloadServiceBuilder, ComponentsBuilder},
    node::{FullNodeTypes, NodeTypes},
    DebugNode, Node, NodeAdapter,
};
use reth_payload_primitives::PayloadTypes;
use reth_provider::{providers::ProviderFactoryBuilder, EthStorage};
use reth_pulsechain_consensus::PulseChainConsensusBuilder;
use reth_pulsechain_evm::PulseChainExecutorBuilder;
use std::sync::Arc;

/// Type configuration for a PulseChain node.
///
/// This is a node type that uses standard Ethereum components with a custom
/// executor that applies PrimordialPulse fork modifications at block 17,233,000.
#[derive(Debug, Default, Clone, Copy)]
#[non_exhaustive]
pub struct PulseChainNode;

impl PulseChainNode {
    /// Returns the components for a PulseChain node with default settings.
    ///
    /// This configures:
    /// - Custom PulseChain executor (applies fork modifications at block 17,233,000)
    /// - Custom PulseChain consensus (uses difficulty-based validation)
    /// - Standard Ethereum components for everything else (pool, network, payload)
    pub fn components<Node>() -> ComponentsBuilder<
        Node,
        EthereumPoolBuilder,
        BasicPayloadServiceBuilder<EthereumPayloadBuilder>,
        EthereumNetworkBuilder,
        PulseChainExecutorBuilder,
        PulseChainConsensusBuilder,
    >
    where
        Node: FullNodeTypes<Types: NodeTypes<ChainSpec = ChainSpec, Primitives = EthPrimitives>>,
        <Node::Types as NodeTypes>::Payload: PayloadTypes<
            BuiltPayload = EthBuiltPayload,
            PayloadAttributes = EthPayloadAttributes,
        >,
    {
        ComponentsBuilder::default()
            .node_types::<Node>()
            .pool(EthereumPoolBuilder::default())
            .executor(PulseChainExecutorBuilder::default())
            .payload(BasicPayloadServiceBuilder::default())
            .network(EthereumNetworkBuilder::default())
            .consensus(PulseChainConsensusBuilder::default())
    }

    /// Instantiates the [`ProviderFactoryBuilder`] for a PulseChain node.
    ///
    /// # Example: Open a ProviderFactory in read-only mode
    ///
    /// ```no_run
    /// use reth_pulsechain_chainspec::PULSECHAIN_MAINNET;
    /// use reth_pulsechain_node::PulseChainNode;
    ///
    /// let factory = PulseChainNode::provider_factory_builder()
    ///     .open_read_only(PULSECHAIN_MAINNET.clone(), "datadir")
    ///     .unwrap();
    /// ```
    pub fn provider_factory_builder() -> ProviderFactoryBuilder<Self> {
        ProviderFactoryBuilder::default()
    }
}

impl NodeTypes for PulseChainNode {
    type Primitives = EthPrimitives;
    type ChainSpec = ChainSpec;
    type Storage = EthStorage;
    type Payload = reth_ethereum::node::EthEngineTypes;
}

impl<N> Node<N> for PulseChainNode
where
    N: FullNodeTypes<Types = Self>,
{
    type ComponentsBuilder = ComponentsBuilder<
        N,
        EthereumPoolBuilder,
        BasicPayloadServiceBuilder<EthereumPayloadBuilder>,
        EthereumNetworkBuilder,
        PulseChainExecutorBuilder,
        PulseChainConsensusBuilder,
    >;

    type AddOns =
        EthereumAddOns<NodeAdapter<N>, EthereumEthApiBuilder, EthereumEngineValidatorBuilder>;

    fn components_builder(&self) -> Self::ComponentsBuilder {
        Self::components()
    }

    fn add_ons(&self) -> Self::AddOns {
        EthereumAddOns::default()
    }
}

impl<N: FullNodeComponents<Types = Self>> DebugNode<N> for PulseChainNode {
    type RpcBlock = alloy_rpc_types_eth::Block;

    fn rpc_to_primitive_block(rpc_block: Self::RpcBlock) -> reth_ethereum_primitives::Block {
        rpc_block.into_consensus().convert_transactions()
    }

    fn local_payload_attributes_builder(
        chain_spec: &Self::ChainSpec,
    ) -> impl PayloadAttributesBuilder<<Self::Payload as PayloadTypes>::PayloadAttributes> {
        LocalPayloadAttributesBuilder::new(Arc::new(chain_spec.clone()))
    }
}
