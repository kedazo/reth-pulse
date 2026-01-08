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

    #[test]
    fn test_pulsechain_fork_id_at_genesis() {
        use reth_ethereum_forks::Head;
        use reth_ethereum_forks::ForkCondition;
        use reth_ethereum_forks::ForkHash;

        // Print all forks for debugging
        println!("\nPulseChain hardforks list:");
        for (fork, condition) in PULSECHAIN_MAINNET.hardforks.forks_iter() {
            match condition {
                ForkCondition::Block(b) => println!("  {:?} -> Block({})", fork, b),
                ForkCondition::Timestamp(t) => println!("  {:?} -> Timestamp({})", fork, t),
                ForkCondition::TTD { activation_block_number, fork_block, total_difficulty } => {
                    println!("  {:?} -> TTD {{ activation: {}, fork_block: {:?}, ttd: {} }}",
                        fork, activation_block_number, fork_block, total_difficulty);
                }
                _ => println!("  {:?} -> {:?}", fork, condition),
            }
        }

        // Manual fork ID calculation to debug
        println!("\nManual fork hash calculation:");
        let mut manual_hash = ForkHash::from(PULSECHAIN_MAINNET.genesis_hash());
        println!("  Start with genesis hash: {:?}", manual_hash);

        // Add each block fork
        for num in [1150000u64, 1920000, 2463000, 2675000, 4370000, 7280000, 9069000, 9200000, 12244000, 12965000, 13773000, 15050000, 17233000] {
            manual_hash += num;
            println!("  After adding block {}: {:?}", num, manual_hash);
        }

        // Add Shanghai timestamp
        manual_hash += 1683786515u64;
        println!("  After adding Shanghai timestamp 1683786515: {:?}", manual_hash);

        // Test WITHOUT block 17233000 (in case peers don't have it)
        println!("\nAlternative calculation WITHOUT block 17233000:");
        let mut alt_hash = ForkHash::from(PULSECHAIN_MAINNET.genesis_hash());
        for num in [1150000u64, 1920000, 2463000, 2675000, 4370000, 7280000, 9069000, 9200000, 12244000, 12965000, 13773000, 15050000] {
            alt_hash += num;
        }
        println!("  After block forks (no 17233000): {:?}", alt_hash);
        alt_hash += 1683786515u64;
        println!("  After adding Shanghai: {:?}", alt_hash);

        // Calculate fork ID at genesis (block 0)
        let head = Head {
            hash: PULSECHAIN_MAINNET.genesis_hash(),
            number: 0,
            timestamp: PULSECHAIN_MAINNET.genesis().timestamp,
            difficulty: PULSECHAIN_MAINNET.genesis().difficulty,
            total_difficulty: PULSECHAIN_MAINNET.genesis().difficulty,
        };

        let fork_id = PULSECHAIN_MAINNET.fork_id(&head);
        println!("\nPulseChain Fork ID at block 0: {:?}", fork_id);

        // Test at block 25M (past all forks including Shanghai)
        let head_late = Head {
            hash: PULSECHAIN_MAINNET.genesis_hash(),
            number: 25_000_000,
            timestamp: 1700000000, // Past Shanghai timestamp
            difficulty: Default::default(),
            total_difficulty: Default::default(),
        };

        let fork_id_late = PULSECHAIN_MAINNET.fork_id(&head_late);
        println!("PulseChain Fork ID at block 25M: {:?}", fork_id_late);
        println!("Expected PulseChain fork hash: 22d523b2");
    }
}
