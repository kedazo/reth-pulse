//! PulseChain EVM and executor implementation.
//!
//! This crate provides a custom block executor for PulseChain that applies
//! fork modifications at the PrimordialPulse block (17,233,000).

#![doc(
    html_logo_url = "https://raw.githubusercontent.com/paradigmxyz/reth/main/assets/reth-docs.png",
    html_favicon_url = "https://avatars0.githubusercontent.com/u/97369466?s=256",
    issue_tracker_base_url = "https://github.com/paradigmxyz/reth/issues/"
)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

mod executor;
mod fork_state;

pub use executor::{PulseChainBlockExecutor, PulseChainEvmConfig, PulseChainExecutorBuilder};
