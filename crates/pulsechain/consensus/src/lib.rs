//! PulseChain consensus implementation.
//!
//! This module provides a consensus engine for PulseChain that uses difficulty-based
//! validation rather than block-number-based validation. This is necessary because
//! PulseChain was Proof-of-Stake from genesis (forked from post-merge Ethereum),
//! but includes a special PrimordialPulse fork block at 17,233,000.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

mod builder;
pub use builder::*;

use alloc::{fmt::Debug, sync::Arc};
use alloy_consensus::EMPTY_OMMER_ROOT_HASH;
use alloy_eips::eip7840::BlobParams;
use reth_chainspec::{EthChainSpec, EthereumHardforks};
use reth_consensus::{Consensus, ConsensusError, FullConsensus, HeaderValidator};
use reth_pulsechain_forks::{ETHEREUM_SHANGHAI_TIME, PULSECHAIN_PRIMORDIAL_BLOCK, PULSECHAIN_SHANGHAI_TIME};
use reth_consensus_common::validation::{
    validate_4844_header_standalone, validate_against_parent_eip1559_base_fee,
    validate_against_parent_gas_limit, validate_against_parent_hash_number,
    validate_against_parent_timestamp, validate_header_base_fee, validate_header_extra_data,
    validate_header_gas,
};
use reth_ethereum_consensus::EthBeaconConsensus;
use reth_execution_types::BlockExecutionResult;
use reth_primitives_traits::{
    Block, BlockHeader, NodePrimitives, RecoveredBlock, SealedBlock, SealedHeader,
};

/// PulseChain consensus implementation.
///
/// This consensus engine wraps the standard Ethereum beacon consensus but uses
/// difficulty-based detection instead of block-number-based detection to determine
/// whether to apply PoW or PoS validation rules.
///
/// # Validation Rules
///
/// - Blocks with `difficulty > 0`: Validated as PoW blocks (ethash rules)
/// - Blocks with `difficulty == 0`: Validated as PoS blocks (beacon rules)
/// - PrimordialPulse block (17,233,000): Has difficulty = 0x20000 (131,072)
///   and transitions the chain to full PoS
#[derive(Debug, Clone)]
pub struct PulseChainBeaconConsensus<ChainSpec> {
    /// Inner Ethereum beacon consensus engine
    inner: Arc<EthBeaconConsensus<ChainSpec>>,
}

impl<ChainSpec> PulseChainBeaconConsensus<ChainSpec>
where
    ChainSpec: EthChainSpec + EthereumHardforks,
{
    /// Create a new PulseChain consensus engine
    pub fn new(chain_spec: Arc<ChainSpec>) -> Self {
        Self { inner: Arc::new(EthBeaconConsensus::new(chain_spec)) }
    }

    /// Returns true if the header should use PoS (beacon) validation rules.
    ///
    /// In PulseChain:
    /// - Headers with difficulty == 0 are PoS blocks
    /// - Headers with difficulty > 0 are PoW blocks (including PrimordialPulse)
    #[inline]
    fn is_pos_header<H>(header: &H) -> bool
    where
        H: BlockHeader,
    {
        header.difficulty().is_zero()
    }

    /// Returns true if Shanghai is active for this block.
    ///
    /// PulseChain has two Shanghai activation times:
    /// - Pre-fork blocks (< 17,233,000): Inherited Ethereum's Shanghai at timestamp 1681338455
    /// - Post-fork blocks (≥ 17,233,000): PulseChain's Shanghai at timestamp 1683786515
    #[inline]
    fn is_shanghai_active<H>(header: &H) -> bool
    where
        H: BlockHeader,
    {
        if header.number() < PULSECHAIN_PRIMORDIAL_BLOCK {
            // Pre-fork: use Ethereum's Shanghai timestamp
            header.timestamp() >= ETHEREUM_SHANGHAI_TIME
        } else {
            // Post-fork: use PulseChain's Shanghai timestamp
            header.timestamp() >= PULSECHAIN_SHANGHAI_TIME
        }
    }
}

impl<ChainSpec, N> FullConsensus<N> for PulseChainBeaconConsensus<ChainSpec>
where
    ChainSpec: Send + Sync + EthChainSpec<Header = N::BlockHeader> + EthereumHardforks + Debug,
    N: NodePrimitives,
{
    fn validate_block_post_execution(
        &self,
        block: &RecoveredBlock<N::Block>,
        result: &BlockExecutionResult<N::Receipt>,
    ) -> Result<(), ConsensusError> {
        FullConsensus::<N>::validate_block_post_execution(self.inner.as_ref(), block, result)
    }
}

impl<B, ChainSpec> Consensus<B> for PulseChainBeaconConsensus<ChainSpec>
where
    B: Block,
    ChainSpec: EthChainSpec<Header = B::Header> + EthereumHardforks + Debug + Send + Sync,
{
    type Error = ConsensusError;

    fn validate_body_against_header(
        &self,
        body: &B::Body,
        header: &SealedHeader<B::Header>,
    ) -> Result<(), Self::Error> {
        Consensus::<B>::validate_body_against_header(self.inner.as_ref(), body, header)
    }

    fn validate_block_pre_execution(&self, block: &SealedBlock<B>) -> Result<(), Self::Error> {
        Consensus::<B>::validate_block_pre_execution(self.inner.as_ref(), block)
    }
}

impl<H, ChainSpec> HeaderValidator<H> for PulseChainBeaconConsensus<ChainSpec>
where
    H: BlockHeader,
    ChainSpec: EthChainSpec<Header = H> + EthereumHardforks + Debug + Send + Sync,
{
    fn validate_header(&self, header: &SealedHeader<H>) -> Result<(), ConsensusError> {
        let header_ref = header.header();
        let is_pow_block = !Self::is_pos_header(header_ref);

        if is_pow_block {
            // Block with non-zero difficulty (either pre-merge PoW or PrimordialPulse at 17,233,000)
            if header_ref.number() == PULSECHAIN_PRIMORDIAL_BLOCK {
                tracing::debug!(
                    target: "consensus::pulsechain",
                    block_number = %header_ref.number(),
                    difficulty = %header_ref.difficulty(),
                    nonce = ?header_ref.nonce(),
                    "Validating PrimordialPulse fork block (special difficulty=0x20000)"
                );
            }
        }

        // PoS-specific validations (skip for PrimordialPulse block which has PoW-like fields)
        if !is_pow_block {
            if !header_ref.nonce().is_some_and(|n| n.is_zero()) {
                return Err(ConsensusError::TheMergeNonceIsNotZero);
            }
            if *header_ref.ommers_hash() != EMPTY_OMMER_ROOT_HASH {
                return Err(ConsensusError::TheMergeOmmerRootIsNotEmpty);
            }
        }

        // Validate standard header fields
        validate_header_extra_data(header_ref, 32)?;
        validate_header_gas(header_ref)?;
        validate_header_base_fee(header_ref, self.inner.chain_spec())?;

        // Validate EIP-4895 withdrawals with custom Shanghai logic
        if Self::is_shanghai_active(header_ref) {
            if header_ref.withdrawals_root().is_none() {
                return Err(ConsensusError::WithdrawalsRootMissing);
            }
        } else if header_ref.withdrawals_root().is_some() {
            return Err(ConsensusError::WithdrawalsRootUnexpected);
        }

        // Validate EIP-4844 blob gas fields
        if self.inner.chain_spec().is_cancun_active_at_timestamp(header_ref.timestamp()) {
            validate_4844_header_standalone(
                header_ref,
                self.inner
                    .chain_spec()
                    .blob_params_at_timestamp(header_ref.timestamp())
                    .unwrap_or_else(BlobParams::cancun),
            )?;
        } else {
            if header_ref.blob_gas_used().is_some() {
                return Err(ConsensusError::BlobGasUsedUnexpected);
            }
            if header_ref.excess_blob_gas().is_some() {
                return Err(ConsensusError::ExcessBlobGasUnexpected);
            }
            if header_ref.parent_beacon_block_root().is_some() {
                return Err(ConsensusError::ParentBeaconBlockRootUnexpected);
            }
        }

        // Validate EIP-7685 requests
        if self.inner.chain_spec().is_prague_active_at_timestamp(header_ref.timestamp()) {
            if header_ref.requests_hash().is_none() {
                return Err(ConsensusError::RequestsHashMissing);
            }
        } else if header_ref.requests_hash().is_some() {
            return Err(ConsensusError::RequestsHashUnexpected);
        }

        Ok(())
    }

    fn validate_header_against_parent(
        &self,
        header: &SealedHeader<H>,
        parent: &SealedHeader<H>,
    ) -> Result<(), ConsensusError> {
        let is_primordial_pulse = parent.difficulty().is_zero() && !header.difficulty().is_zero();
        let is_after_primordial = !parent.difficulty().is_zero() && header.difficulty().is_zero();

        if is_primordial_pulse {
            // Block 17,233,000: transition from difficulty=0 to difficulty=0x20000
            tracing::debug!(
                target: "consensus::pulsechain",
                block_number = %header.number(),
                "Validating PrimordialPulse fork block transition (0 -> 131072)"
            );
        } else if is_after_primordial {
            // Block 17,233,001: transition from difficulty=0x20000 back to difficulty=0
            tracing::debug!(
                target: "consensus::pulsechain",
                block_number = %header.number(),
                "Validating post-PrimordialPulse transition (131072 -> 0)"
            );
        }

        // For PrimordialPulse transitions, we need custom validation that doesn't check
        // difficulty transitions, since the inner validator expects difficulty to stay at 0 post-merge
        if is_primordial_pulse || is_after_primordial {
            // Manually validate the critical parent relationships without difficulty checks
            validate_against_parent_hash_number(header.header(), parent)?;
            validate_against_parent_timestamp(header.header(), parent.header())?;
            validate_against_parent_gas_limit(header, parent, self.inner.chain_spec())?;
            validate_against_parent_eip1559_base_fee(
                header.header(),
                parent.header(),
                self.inner.chain_spec(),
            )?;

            return Ok(());
        }

        // For all other blocks, use standard parent validation
        HeaderValidator::<H>::validate_header_against_parent(self.inner.as_ref(), header, parent)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reth_pulsechain_chainspec::PULSECHAIN_MAINNET;

    #[test]
    fn test_pulsechain_consensus_creation() {
        let consensus = PulseChainBeaconConsensus::new(PULSECHAIN_MAINNET.clone());
        assert!(Arc::strong_count(&consensus.inner) == 1);
    }

    #[test]
    fn test_is_pos_header_detection() {
        use alloy_primitives::U256;
        use reth_primitives::Header;

        // Headers with difficulty == 0 should be PoS
        assert!(PulseChainBeaconConsensus::<reth_chainspec::ChainSpec>::is_pos_header(&Header {
            difficulty: U256::ZERO,
            ..Default::default()
        }));

        // Headers with difficulty > 0 should not be PoS
        assert!(!PulseChainBeaconConsensus::<reth_chainspec::ChainSpec>::is_pos_header(&Header {
            difficulty: U256::from(0x20000), // PrimordialPulse difficulty
            ..Default::default()
        }));
    }
}
