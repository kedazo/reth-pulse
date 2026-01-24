//! Pulse-Reth hard forks.
//!
//! This defines the [`ChainHardforks`] for PulseChain mainnet.
//! PulseChain is a fork of Ethereum that occurred at block 17,233,000 (PrimordialPulse block).

#![doc(
    html_logo_url = "https://raw.githubusercontent.com/paradigmxyz/reth/main/assets/reth-docs.png",
    html_favicon_url = "https://avatars0.githubusercontent.com/u/97369466?s=256",
    issue_tracker_base_url = "https://github.com/paradigmxyz/reth/issues/"
)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![cfg_attr(not(feature = "std"), no_std)]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]

extern crate alloc;

use alloc::{boxed::Box, vec};
use alloy_primitives::U256;
use core::fmt;
use once_cell::sync::Lazy as LazyLock;
use reth_ethereum_forks::{ChainHardforks, EthereumHardfork, ForkCondition, Hardfork};

/// PulseChain-specific hardfork
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PulseChainHardfork {
    /// PrimordialPulse fork at block 17,233,000
    ///
    /// This is the block where PulseChain forks from Ethereum mainnet.
    /// At this block:
    /// - Chain ID changes from 1 (Ethereum) to 369 (PulseChain)
    /// - Sacrifice credits are applied to accounts
    /// - Deposit contracts are replaced
    PrimordialPulse,
}

impl fmt::Display for PulseChainHardfork {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Self::PrimordialPulse => "PrimordialPulse",
            }
        )
    }
}

impl Hardfork for PulseChainHardfork {
    fn name(&self) -> &'static str {
        match self {
            Self::PrimordialPulse => "PrimordialPulse",
        }
    }
}

/// PulseChain mainnet list of hardforks.
///
/// PulseChain follows Ethereum mainnet history up to block 17,233,000 (PrimordialPulse),
/// then forks to create the PulseChain network with chain ID 369.
///
/// The PrimordialPulse block (17,233,000) is where:
/// - Chain ID changes from 1 (Ethereum) to 369 (PulseChain)
/// - Sacrifice credits are applied to accounts
/// - Deposit contracts are replaced
/// - Terminal Total Difficulty is reached with an offset
///
/// Note: Only Ethereum hardforks active at block 17,233,000 are included.
/// Shanghai and later forks are PulseChain-specific activations.
pub static PULSECHAIN_MAINNET_HARDFORKS: LazyLock<ChainHardforks> = LazyLock::new(|| {
    // PulseChain follows Ethereum mainnet history EXACTLY from block 0 to 17,233,000
    // All Ethereum hardforks at their original block numbers
    ChainHardforks::new(vec![
        (EthereumHardfork::Frontier.boxed(), ForkCondition::Block(0)),
        (EthereumHardfork::Homestead.boxed(), ForkCondition::Block(1_150_000)),
        (EthereumHardfork::Dao.boxed(), ForkCondition::Block(1_920_000)),
        (EthereumHardfork::Tangerine.boxed(), ForkCondition::Block(2_463_000)),
        (EthereumHardfork::SpuriousDragon.boxed(), ForkCondition::Block(2_675_000)),
        (EthereumHardfork::Byzantium.boxed(), ForkCondition::Block(4_370_000)),
        (EthereumHardfork::Constantinople.boxed(), ForkCondition::Block(7_280_000)),
        (EthereumHardfork::Petersburg.boxed(), ForkCondition::Block(7_280_000)),
        (EthereumHardfork::Istanbul.boxed(), ForkCondition::Block(9_069_000)),
        (EthereumHardfork::MuirGlacier.boxed(), ForkCondition::Block(9_200_000)),
        (EthereumHardfork::Berlin.boxed(), ForkCondition::Block(12_244_000)),
        (EthereumHardfork::London.boxed(), ForkCondition::Block(12_965_000)),
        (EthereumHardfork::ArrowGlacier.boxed(), ForkCondition::Block(13_773_000)),
        (EthereumHardfork::GrayGlacier.boxed(), ForkCondition::Block(15_050_000)),
        // Paris (The Merge) at Ethereum's original TTD
        // Block 15,537,394 is the first PoS block (where difficulty == 0)
        // PulseChain inherits this merge since the fork happened AFTER the merge
        (
            EthereumHardfork::Paris.boxed(),
            ForkCondition::TTD {
                activation_block_number: 15_537_394,
                fork_block: None,
                total_difficulty: U256::from_limbs([
                    (ETHEREUM_MAINNET_TTD & 0xFFFFFFFFFFFFFFFF) as u64,
                    (ETHEREUM_MAINNET_TTD >> 64) as u64,
                    0,
                    0,
                ]),
            },
        ),
        // PrimordialPulse fork at block 17,233,000
        // This is the actual fork point where PulseChain diverges from Ethereum
        // CRITICAL: This must be included for correct fork ID calculation
        (
            Box::new(PulseChainHardfork::PrimordialPulse),
            ForkCondition::Block(PULSECHAIN_PRIMORDIAL_BLOCK),
        ),
        // Shanghai: Ethereum's Shanghai timestamp (April 12, 2023)
        // This timestamp was already active when PulseChain forked, so we use Ethereum's time
        // CRITICAL: This must come AFTER PrimordialPulse in the list for correct fork ID
        (EthereumHardfork::Shanghai.boxed(), ForkCondition::Timestamp(ETHEREUM_SHANGHAI_TIME)),
    ])
});

/// PulseChain mainnet chain ID
pub const PULSECHAIN_CHAIN_ID: u64 = 369;

/// Ethereum mainnet chain ID (used pre-fork)
pub const ETHEREUM_CHAIN_ID: u64 = 1;

/// PrimordialPulse fork block number
///
/// This is the block where PulseChain forks from Ethereum mainnet.
/// At this block:
/// - Chain ID changes from 1 to 369
/// - Sacrifice credits are applied
/// - Deposit contracts are replaced
pub const PULSECHAIN_PRIMORDIAL_BLOCK: u64 = 17_233_000;

/// TTD offset added to Ethereum's TTD
pub const PULSECHAIN_TTD_OFFSET: u64 = 131_072;

/// Ethereum mainnet's final TTD (the merge)
/// This is the TTD at which Ethereum transitioned to Proof-of-Stake at block 15,537,393
pub const ETHEREUM_MAINNET_TTD: u128 = 58_750_003_716_598_352_816_469;

/// Terminal Total Difficulty for PulseChain
///
/// Calculated as: Ethereum's last actual TTD + 131,072
/// = 58_750_003_716_598_352_816_469 + 131_072
/// = 58_750_003_716_598_352_947_541
pub const PULSECHAIN_TTD: U256 = U256::from_limbs([
    ((ETHEREUM_MAINNET_TTD + PULSECHAIN_TTD_OFFSET as u128) & 0xFFFFFFFFFFFFFFFF) as u64,
    ((ETHEREUM_MAINNET_TTD + PULSECHAIN_TTD_OFFSET as u128) >> 64) as u64,
    0,
    0,
]);

/// Shanghai activation timestamp (inherited from Ethereum mainnet)
/// April 12, 2023, 22:27:35 UTC
/// This is Ethereum's Shanghai timestamp, which PulseChain inherited since the fork
/// occurred after Shanghai was already active on Ethereum
pub const ETHEREUM_SHANGHAI_TIME: u64 = 1681338455;

/// PulseChain-specific Shanghai timestamp (unused - kept for reference)
/// May 11, 2023, 06:01:55 UTC
/// Note: Not used because PulseChain forked from Ethereum AFTER Shanghai was already active
pub const PULSECHAIN_SHANGHAI_TIME: u64 = 1683786515;

/// Helper function to determine if a block number is before the PrimordialPulse fork
#[inline]
pub const fn is_before_primordial_pulse(block_number: u64) -> bool {
    block_number < PULSECHAIN_PRIMORDIAL_BLOCK
}

/// Helper function to determine if a block number is at or after the PrimordialPulse fork
#[inline]
pub const fn is_at_or_after_primordial_pulse(block_number: u64) -> bool {
    block_number >= PULSECHAIN_PRIMORDIAL_BLOCK
}

/// Get the effective chain ID for a given block number on PulseChain
///
/// Returns:
/// - 1 (Ethereum) for blocks before PrimordialPulse (< 17,233,000)
/// - 369 (PulseChain) for blocks at or after PrimordialPulse (>= 17,233,000)
#[inline]
pub const fn get_effective_chain_id(block_number: u64) -> u64 {
    if is_before_primordial_pulse(block_number) {
        ETHEREUM_CHAIN_ID
    } else {
        PULSECHAIN_CHAIN_ID
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_effective_chain_id() {
        // Pre-fork blocks should use Ethereum chain ID
        assert_eq!(get_effective_chain_id(0), ETHEREUM_CHAIN_ID);
        assert_eq!(get_effective_chain_id(17_232_999), ETHEREUM_CHAIN_ID);

        // Fork block and after should use PulseChain chain ID
        assert_eq!(get_effective_chain_id(17_233_000), PULSECHAIN_CHAIN_ID);
        assert_eq!(get_effective_chain_id(17_233_001), PULSECHAIN_CHAIN_ID);
        assert_eq!(get_effective_chain_id(20_000_000), PULSECHAIN_CHAIN_ID);
    }

    #[test]
    fn test_primordial_pulse_detection() {
        assert!(is_before_primordial_pulse(0));
        assert!(is_before_primordial_pulse(17_232_999));
        assert!(!is_before_primordial_pulse(17_233_000));
        assert!(!is_before_primordial_pulse(17_233_001));

        assert!(!is_at_or_after_primordial_pulse(17_232_999));
        assert!(is_at_or_after_primordial_pulse(17_233_000));
        assert!(is_at_or_after_primordial_pulse(17_233_001));
    }

    #[test]
    fn test_ttd_value() {
        // Verify TTD is correctly calculated
        // Should be 58_750_003_716_598_352_947_541
        let expected_ttd =
            U256::from(58_750_003_716_598_352_816_469u128) + U256::from(PULSECHAIN_TTD_OFFSET);
        assert_eq!(PULSECHAIN_TTD, expected_ttd);
    }

    #[test]
    fn test_hardforks_ordered() {
        let hardforks = &*PULSECHAIN_MAINNET_HARDFORKS;

        // Verify we have the expected number of forks
        assert!(!hardforks.is_empty());

        // Verify key forks exist
        assert!(hardforks.fork(EthereumHardfork::London).active_at_block(12_965_000));
        // Paris activated at Ethereum's merge - block 15,537,394 is the first PoS block
        assert!(hardforks.fork(EthereumHardfork::Paris).active_at_block(15_537_394));
        // Block 15,537,393 (last PoW block) should NOT use Paris rules
        assert!(!hardforks.fork(EthereumHardfork::Paris).active_at_block(15_537_393));
        // Shanghai uses Ethereum's timestamp since PulseChain forked after Shanghai
        assert!(hardforks
            .fork(EthereumHardfork::Shanghai)
            .active_at_timestamp(ETHEREUM_SHANGHAI_TIME));
    }

    #[test]
    fn test_primordial_pulse_hardfork() {
        let hardforks = &*PULSECHAIN_MAINNET_HARDFORKS;

        // Verify PrimordialPulse fork exists and is at the correct block
        assert!(hardforks.fork(PulseChainHardfork::PrimordialPulse).active_at_block(17_233_000));
        assert!(!hardforks.fork(PulseChainHardfork::PrimordialPulse).active_at_block(17_232_999));
    }
}
