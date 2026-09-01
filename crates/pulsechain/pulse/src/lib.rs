//! PulseChain fork logic implementation.
//!
//! This crate provides the implementation of PulseChain-specific fork logic,
//! including sacrifice credits and deposit contract replacement.

#![doc(
    html_logo_url = "https://raw.githubusercontent.com/paradigmxyz/reth/main/assets/reth-docs.png",
    html_favicon_url = "https://avatars0.githubusercontent.com/u/97369466?s=256",
    issue_tracker_base_url = "https://github.com/paradigmxyz/reth/issues/"
)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

mod deposit_contract;
mod fork;
mod sacrifice_credits;

pub use deposit_contract::{
    get_deposit_contract_data, get_nil_contract_bytecode, DepositContractData,
    DEPOSIT_CONTRACT_BYTECODE, DEPOSIT_CONTRACT_STORAGE, ETHEREUM_DEPOSIT_CONTRACT,
    NIL_CONTRACT_BYTECODE, PULSECHAIN_DEPOSIT_CONTRACT,
};
pub use fork::{
    get_fork_instructions, get_primordial_pulse_fork, ForkInstructions, PrimordialPulseFork,
};
pub use sacrifice_credits::{
    get_mainnet_sacrifice_credits, mainnet_sacrifice_credits_count, parse_sacrifice_credits,
    SacrificeCredit,
};
