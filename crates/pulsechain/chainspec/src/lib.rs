//! PulseChain chain specifications.
//!
//! This crate provides the chain specification for PulseChain mainnet.

#![doc(
    html_logo_url = "https://raw.githubusercontent.com/paradigmxyz/reth/main/assets/reth-docs.png",
    html_favicon_url = "https://avatars0.githubusercontent.com/u/97369466?s=256",
    issue_tracker_base_url = "https://github.com/paradigmxyz/reth/issues/"
)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

use alloc::sync::Arc;
use alloy_chains::Chain;
use alloy_genesis::Genesis;
use alloy_primitives::{address, b256};
use once_cell::sync::Lazy as LazyLock;
use reth_chainspec::{BaseFeeParams, BaseFeeParamsKind, ChainSpec, DepositContract};
use reth_primitives_traits::SealedHeader;
use reth_pulsechain_forks::*;

mod bootnodes;
pub use bootnodes::*;

/// PulseChain mainnet deposit contract address
///
/// This is deployed at the PrimordialPulse fork block (17,233,000)
pub const PULSECHAIN_DEPOSIT_CONTRACT_ADDRESS: alloy_primitives::Address =
    address!("3693693693693693693693693693693693693693");

/// PulseChain mainnet deposit contract deployment block
pub const PULSECHAIN_DEPOSIT_CONTRACT_BLOCK: u64 = PULSECHAIN_PRIMORDIAL_BLOCK;

/// PulseChain mainnet deposit contract deployment transaction hash
///
/// Note: This is the hash of the transaction that deployed the deposit contract
/// during the PrimordialPulse fork
pub const PULSECHAIN_DEPOSIT_CONTRACT_DEPLOYMENT_HASH: alloy_primitives::B256 =
    b256!("649bbc62d0e31342afea4e5cd82d4049e7e1ee912fc0889aa790803be39038c5");

/// PulseChain mainnet deposit contract
pub static PULSECHAIN_DEPOSIT_CONTRACT: LazyLock<DepositContract> = LazyLock::new(|| {
    DepositContract::new(
        PULSECHAIN_DEPOSIT_CONTRACT_ADDRESS,
        PULSECHAIN_DEPOSIT_CONTRACT_BLOCK,
        PULSECHAIN_DEPOSIT_CONTRACT_DEPLOYMENT_HASH,
    )
});

/// PulseChain mainnet genesis hash.
///
/// PulseChain uses Ethereum's genesis block, so this is the same as Ethereum mainnet.
pub const PULSECHAIN_GENESIS_HASH: alloy_primitives::B256 =
    b256!("d4e56740f876aef8c010b86a40d5f56745a118d0906a34e69aec8c0db1cb8fa3");

/// The PulseChain mainnet spec
pub static PULSECHAIN_MAINNET: LazyLock<Arc<ChainSpec>> = LazyLock::new(|| {
    // Load Ethereum mainnet genesis (PulseChain uses the same genesis)
    let genesis: Genesis = serde_json::from_str(include_str!("../res/genesis/pulsechain.json"))
        .expect("Can't deserialize PulseChain genesis json");

    // Use PulseChain hardforks
    let hardforks = PULSECHAIN_MAINNET_HARDFORKS.clone();

    // Make genesis header using helper function from reth-chainspec
    let genesis_header = reth_chainspec::make_genesis_header(&genesis, &hardforks);

    let mut spec = ChainSpec {
        // PulseChain uses a custom chain with ID 369
        chain: Chain::from_id(PULSECHAIN_CHAIN_ID),
        // Genesis header uses Ethereum's genesis hash (PulseChain shares Ethereum's genesis)
        genesis_header: SealedHeader::new(genesis_header, PULSECHAIN_GENESIS_HASH),
        genesis,
        // Paris activated at PrimordialPulse block with custom TTD
        paris_block_and_final_difficulty: Some((PULSECHAIN_PRIMORDIAL_BLOCK, PULSECHAIN_TTD)),
        hardforks,
        // PulseChain deposit contract
        deposit_contract: Some(*PULSECHAIN_DEPOSIT_CONTRACT),
        // Use Ethereum's base fee parameters
        base_fee_params: BaseFeeParamsKind::Constant(BaseFeeParams::ethereum()),
        // Default prune delete limit
        prune_delete_limit: 10_000,
        // No blob params overrides (use defaults)
        ..Default::default()
    };

    // Enable DAO fork support (inherited from Ethereum)
    spec.genesis.config.dao_fork_support = true;

    spec.into()
});

// Note: EthChainSpec is automatically implemented for ChainSpec<H>
// We don't need to implement it manually

#[cfg(test)]
mod tests {
    use super::*;
    use reth_chainspec::EthChainSpec;
    use reth_ethereum_forks::EthereumHardfork;

    #[test]
    fn test_pulsechain_chain_id() {
        assert_eq!(PULSECHAIN_MAINNET.chain.id(), PULSECHAIN_CHAIN_ID);
    }

    #[test]
    fn test_pulsechain_genesis_hash() {
        // PulseChain uses Ethereum's genesis hash
        assert_eq!(PULSECHAIN_MAINNET.genesis_hash(), PULSECHAIN_GENESIS_HASH);
    }

    #[test]
    fn test_pulsechain_deposit_contract() {
        let deposit_contract =
            PULSECHAIN_MAINNET.deposit_contract().expect("deposit contract should exist");
        assert_eq!(deposit_contract.address, PULSECHAIN_DEPOSIT_CONTRACT_ADDRESS);
        assert_eq!(deposit_contract.block, PULSECHAIN_DEPOSIT_CONTRACT_BLOCK);
    }

    #[test]
    fn test_pulsechain_hardforks() {
        let spec = &*PULSECHAIN_MAINNET;

        // Verify London is active at its activation block
        assert!(spec.hardforks.fork(EthereumHardfork::London).active_at_block(12_965_000));

        // Verify Paris is active at PrimordialPulse block
        assert!(spec
            .hardforks
            .fork(EthereumHardfork::Paris)
            .active_at_block(PULSECHAIN_PRIMORDIAL_BLOCK));

        // Verify Shanghai is active after its timestamp
        assert!(spec
            .hardforks
            .fork(EthereumHardfork::Shanghai)
            .active_at_timestamp(PULSECHAIN_SHANGHAI_TIME));
    }

    #[test]
    fn test_pulsechain_not_ethereum() {
        assert!(!PULSECHAIN_MAINNET.is_ethereum());
    }

    #[test]
    fn test_pulsechain_paris_block() {
        let (block, ttd) =
            PULSECHAIN_MAINNET.paris_block_and_final_difficulty.expect("Paris block should be set");
        assert_eq!(block, PULSECHAIN_PRIMORDIAL_BLOCK);
        assert_eq!(ttd, PULSECHAIN_TTD);
    }
}
