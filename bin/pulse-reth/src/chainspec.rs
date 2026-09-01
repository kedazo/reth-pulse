//! PulseChain chainspec parser for CLI.

use reth_chainspec::ChainSpec;
use reth_cli::chainspec::{parse_genesis, ChainSpecParser};
use reth_pulsechain_chainspec::PULSECHAIN_MAINNET;
use std::sync::Arc;

/// Chains supported by pulse-reth.
const SUPPORTED_CHAINS: &[&str] = &["pulsechain"];

/// Clap value parser for PulseChain [`ChainSpec`]s.
///
/// The value parser matches either a known chain, the path
/// to a json file, or a json formatted string in-memory. The json needs to be a Genesis struct.
fn chain_value_parser(s: &str) -> eyre::Result<Arc<ChainSpec>, eyre::Error> {
    Ok(match s {
        "pulsechain" => PULSECHAIN_MAINNET.clone(),
        _ => Arc::new(parse_genesis(s)?.into()),
    })
}

/// PulseChain chain specification parser.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub(crate) struct PulseChainSpecParser;

impl ChainSpecParser for PulseChainSpecParser {
    type ChainSpec = ChainSpec;

    const SUPPORTED_CHAINS: &'static [&'static str] = SUPPORTED_CHAINS;

    fn parse(s: &str) -> eyre::Result<Arc<ChainSpec>> {
        chain_value_parser(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_known_chain_spec() {
        for &chain in PulseChainSpecParser::SUPPORTED_CHAINS {
            assert!(<PulseChainSpecParser as ChainSpecParser>::parse(chain).is_ok());
        }
    }
}
