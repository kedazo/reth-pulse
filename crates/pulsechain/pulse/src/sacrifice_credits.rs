///! PulseChain sacrifice credits implementation
///!
///! Applies balance credits to sacrifice participants at the PrimordialPulse fork block.
use alloc::{format, vec::Vec};
use alloy_primitives::{Address, U256};
use reth_execution_errors::{BlockExecutionError, InternalBlockExecutionError};

/// Embedded sacrifice credits data for PulseChain mainnet
///
/// Format: [1-byte record length][20-byte address][N-byte balance]
/// Source: https://gitlab.com/pulsechaincom/compressed-allocations/-/tags/Mainnet
const MAINNET_SACRIFICE_CREDITS: &[u8] = include_bytes!("../sacrifice_credits_mainnet.bin");

/// A single sacrifice credit entry
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SacrificeCredit {
    /// The address receiving the credit
    pub address: Address,
    /// The amount of credit to apply
    pub credit: U256,
}

/// Parse sacrifice credits from the binary format
///
/// The binary format is:
/// - 1 byte: record length (20 + balance bytes)
/// - 20 bytes: address
/// - N bytes: balance (big-endian)
pub fn parse_sacrifice_credits(data: &[u8]) -> Result<Vec<SacrificeCredit>, BlockExecutionError> {
    let mut credits = Vec::new();
    let mut ptr = 0;

    while ptr < data.len() {
        // Read record length
        if ptr >= data.len() {
            return Err(BlockExecutionError::Internal(InternalBlockExecutionError::Other(
                "Unexpected end of sacrifice credits data".into(),
            )));
        }
        let byte_count = data[ptr] as usize;
        ptr += 1;

        // Validate we have enough data
        if ptr + byte_count > data.len() {
            return Err(BlockExecutionError::Internal(
                InternalBlockExecutionError::Other(
                    format!(
                        "Invalid sacrifice credits: not enough data for record (expected {}, remaining {})",
                        byte_count,
                        data.len() - ptr
                    )
                    .into(),
                ),
            ));
        }

        // Extract record
        let record = &data[ptr..ptr + byte_count];
        ptr += byte_count;

        // Parse address (first 20 bytes)
        if record.len() < 20 {
            return Err(BlockExecutionError::Internal(InternalBlockExecutionError::Other(
                format!("Invalid sacrifice credit record: too short ({})", record.len()).into(),
            )));
        }

        let address = Address::from_slice(&record[0..20]);

        // Parse balance (remaining bytes, big-endian)
        let balance_bytes = &record[20..];
        let credit = U256::from_be_slice(balance_bytes);

        credits.push(SacrificeCredit { address, credit });
    }

    Ok(credits)
}

/// Get the mainnet sacrifice credits
///
/// Returns all sacrifice credit entries that should be applied at the PrimordialPulse fork.
pub fn get_mainnet_sacrifice_credits() -> Result<Vec<SacrificeCredit>, BlockExecutionError> {
    parse_sacrifice_credits(MAINNET_SACRIFICE_CREDITS)
}

/// Get the total number of sacrifice credits
pub fn mainnet_sacrifice_credits_count() -> Result<usize, BlockExecutionError> {
    let credits = get_mainnet_sacrifice_credits()?;
    Ok(credits.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_sacrifice_credits() {
        // Parse the embedded mainnet sacrifice credits
        let result = parse_sacrifice_credits(MAINNET_SACRIFICE_CREDITS);
        assert!(result.is_ok(), "Failed to parse sacrifice credits: {:?}", result.err());

        let credits = result.unwrap();

        // Verify we have credits
        assert!(credits.len() > 0, "Expected at least one sacrifice credit");

        // Verify all credits have non-zero amounts
        for credit in &credits {
            assert!(
                credit.credit > U256::ZERO,
                "Credit amount should be non-zero for {}",
                credit.address
            );
        }

        println!("Parsed {} sacrifice credits", credits.len());
        println!("First credit: {} -> {}", credits[0].address, credits[0].credit);
    }

    #[test]
    fn test_parse_sample_credits() {
        // Create a sample binary with 2 credits
        let mut data = Vec::new();

        // Credit 1: address 0x0000...0001, balance 1000
        data.push(20 + 2); // record length: 20 bytes (address) + 2 bytes (balance)
        data.extend_from_slice(&[0u8; 19]); // 19 zero bytes
        data.push(1); // last byte of address
        data.extend_from_slice(&1000u16.to_be_bytes()); // balance = 1000

        // Credit 2: address 0x0000...0002, balance 2000
        data.push(20 + 2);
        data.extend_from_slice(&[0u8; 19]);
        data.push(2);
        data.extend_from_slice(&2000u16.to_be_bytes());

        let credits = parse_sacrifice_credits(&data).unwrap();
        assert_eq!(credits.len(), 2);

        assert_eq!(
            credits[0].address,
            Address::from([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1])
        );
        assert_eq!(credits[0].credit, U256::from(1000));

        assert_eq!(
            credits[1].address,
            Address::from([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2])
        );
        assert_eq!(credits[1].credit, U256::from(2000));
    }

    #[test]
    fn test_parse_invalid_data() {
        // Test with truncated data
        let data = vec![10]; // Says 10 bytes but no data
        let result = parse_sacrifice_credits(&data);
        assert!(result.is_err());

        // Test with incomplete record
        let data = vec![5, 1, 2]; // Says 5 bytes but only 2 provided
        let result = parse_sacrifice_credits(&data);
        assert!(result.is_err());
    }
}
