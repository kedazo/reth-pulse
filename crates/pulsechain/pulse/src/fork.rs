///! PulseChain PrimordialPulse fork handler
///!
///! Provides the fork modifications data and specifications that need to be applied
///! at the PrimordialPulse fork block (17,233,000).
///!
///! This module provides all the data and specifications needed for the fork.
///! The actual integration with the EVM State will be done in the custom block executor
///! (Phase 3: Consensus Layer Modifications).
use crate::{
    get_deposit_contract_data, get_mainnet_sacrifice_credits, get_nil_contract_bytecode,
    DepositContractData, SacrificeCredit, ETHEREUM_DEPOSIT_CONTRACT, PULSECHAIN_DEPOSIT_CONTRACT,
};
use alloc::vec::Vec;
use alloy_primitives::{Address, Bytes};
use reth_execution_errors::BlockExecutionError;

/// Fork modifications that need to be applied at the PrimordialPulse block
#[derive(Debug, Clone)]
pub struct PrimordialPulseFork {
    /// Sacrifice credits to apply (address -> balance increase)
    pub sacrifice_credits: Vec<SacrificeCredit>,
    /// Ethereum deposit contract to destroy
    pub ethereum_deposit_contract: Address,
    /// Bytecode to set on destroyed Ethereum deposit contract
    pub nil_contract_bytecode: Bytes,
    /// PulseChain deposit contract to deploy
    pub pulsechain_deposit_contract: Address,
    /// PulseChain deposit contract data (bytecode + storage)
    pub deposit_contract_data: DepositContractData,
}

impl PrimordialPulseFork {
    /// Create a new PrimordialPulseFork with all mainnet fork data
    pub fn mainnet() -> Result<Self, BlockExecutionError> {
        Ok(Self {
            sacrifice_credits: get_mainnet_sacrifice_credits()?,
            ethereum_deposit_contract: ETHEREUM_DEPOSIT_CONTRACT,
            nil_contract_bytecode: get_nil_contract_bytecode(),
            pulsechain_deposit_contract: PULSECHAIN_DEPOSIT_CONTRACT,
            deposit_contract_data: get_deposit_contract_data(),
        })
    }

    /// Get the number of sacrifice credits
    pub fn sacrifice_credits_count(&self) -> usize {
        self.sacrifice_credits.len()
    }

    /// Get the number of deposit contract storage slots
    pub fn deposit_storage_slots_count(&self) -> usize {
        self.deposit_contract_data.storage.len()
    }
}

/// Instructions for applying the PrimordialPulse fork
///
/// This provides a checklist of what needs to be done at the fork block.
/// These instructions will be implemented in the custom block executor.
#[derive(Debug, Clone)]
pub struct ForkInstructions {
    /// Step 1: Apply sacrifice credits
    pub step_1_sacrifice_credits: SacrificeCreditsInstructions,
    /// Step 2: Replace deposit contract
    pub step_2_deposit_contract: DepositContractInstructions,
}

/// Instructions for applying sacrifice credits
#[derive(Debug, Clone)]
pub struct SacrificeCreditsInstructions {
    /// Number of credits to apply
    pub count: usize,
    /// Instructions:
    /// 1. For each sacrifice credit:
    ///    - Load the account (create if it doesn't exist)
    ///    - Add the credit amount to account balance
    ///    - Mark account as touched
    pub instructions: &'static str,
}

/// Instructions for replacing the deposit contract
#[derive(Debug, Clone)]
pub struct DepositContractInstructions {
    /// Ethereum deposit contract address to destroy
    pub eth_contract: Address,
    /// PulseChain deposit contract address to create
    pub pulse_contract: Address,
    /// Number of storage slots to initialize
    pub storage_slots_count: usize,
    /// Instructions:
    /// 1. Self-destruct Ethereum deposit contract
    /// 2. Set Ethereum deposit contract code to nil contract
    /// 3. Clear any balance on PulseChain deposit contract
    /// 4. Deploy PulseChain deposit contract bytecode
    /// 5. Set nonce to 0
    /// 6. Initialize 31 storage slots (0x22-0x40) with zero hash array
    pub instructions: &'static str,
}

impl ForkInstructions {
    /// Get fork instructions for mainnet
    pub fn mainnet() -> Result<Self, BlockExecutionError> {
        let fork = PrimordialPulseFork::mainnet()?;

        Ok(Self {
            step_1_sacrifice_credits: SacrificeCreditsInstructions {
                count: fork.sacrifice_credits_count(),
                instructions: "\
1. Get all sacrifice credits from PrimordialPulseFork::mainnet()
2. For each SacrificeCredit { address, credit }:
   - Load the account (creates if doesn't exist)
   - Add credit to account.balance
   - Mark account as touched
3. Log completion",
            },
            step_2_deposit_contract: DepositContractInstructions {
                eth_contract: ETHEREUM_DEPOSIT_CONTRACT,
                pulse_contract: PULSECHAIN_DEPOSIT_CONTRACT,
                storage_slots_count: 31,
                instructions: "\
1. Load Ethereum deposit contract account
   - Mark as self-destructed
   - Set code to nil contract bytecode
2. Load PulseChain deposit contract account
   - Clear any existing balance
   - Set code to PulseChain deposit contract bytecode
   - Set nonce to 0
   - Initialize 31 storage slots (0x22-0x40) with zero hash array
   - Mark as touched
3. Log completion",
            },
        })
    }
}

/// Get the complete PrimordialPulse fork data for mainnet
///
/// This is the main entry point for accessing all fork modifications.
/// Returns all data needed to apply the fork at block 17,233,000.
pub fn get_primordial_pulse_fork() -> Result<PrimordialPulseFork, BlockExecutionError> {
    PrimordialPulseFork::mainnet()
}

/// Get the fork instructions
///
/// Returns step-by-step instructions for implementing the fork in a block executor.
pub fn get_fork_instructions() -> Result<ForkInstructions, BlockExecutionError> {
    ForkInstructions::mainnet()
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::U256;

    #[test]
    fn test_get_primordial_pulse_fork() {
        let fork = get_primordial_pulse_fork().unwrap();

        // Verify sacrifice credits
        assert_eq!(fork.sacrifice_credits_count(), 292217, "Expected 292,217 sacrifice credits");

        // Verify deposit contract addresses
        assert_eq!(fork.ethereum_deposit_contract, ETHEREUM_DEPOSIT_CONTRACT);
        assert_eq!(fork.pulsechain_deposit_contract, PULSECHAIN_DEPOSIT_CONTRACT);

        // Verify deposit contract data
        assert_eq!(fork.deposit_storage_slots_count(), 31, "Expected 31 storage slots");
        assert!(!fork.deposit_contract_data.bytecode.is_empty());
        assert!(!fork.nil_contract_bytecode.is_empty());
    }

    #[test]
    fn test_get_fork_instructions() {
        let instructions = get_fork_instructions().unwrap();

        // Verify sacrifice credits instructions
        assert_eq!(instructions.step_1_sacrifice_credits.count, 292217);
        assert!(!instructions.step_1_sacrifice_credits.instructions.is_empty());

        // Verify deposit contract instructions
        assert_eq!(instructions.step_2_deposit_contract.eth_contract, ETHEREUM_DEPOSIT_CONTRACT);
        assert_eq!(
            instructions.step_2_deposit_contract.pulse_contract,
            PULSECHAIN_DEPOSIT_CONTRACT
        );
        assert_eq!(instructions.step_2_deposit_contract.storage_slots_count, 31);
        assert!(!instructions.step_2_deposit_contract.instructions.is_empty());
    }

    #[test]
    fn test_primordial_pulse_fork_clone() {
        let fork = get_primordial_pulse_fork().unwrap();
        let cloned = fork.clone();

        assert_eq!(fork.sacrifice_credits_count(), cloned.sacrifice_credits_count());
        assert_eq!(fork.ethereum_deposit_contract, cloned.ethereum_deposit_contract);
    }

    #[test]
    fn test_sacrifice_credits_data_available() {
        let fork = get_primordial_pulse_fork().unwrap();

        // Verify first sacrifice credit has valid data
        assert!(!fork.sacrifice_credits.is_empty());
        assert!(fork.sacrifice_credits[0].credit > U256::ZERO);
    }

    #[test]
    fn test_deposit_contract_data_complete() {
        let fork = get_primordial_pulse_fork().unwrap();

        // Verify bytecode
        assert!(fork.deposit_contract_data.bytecode.len() > 0);

        // Verify storage slots
        assert_eq!(fork.deposit_contract_data.storage.len(), 31);

        // Verify storage slots are in correct range (0x22-0x40)
        for (slot, _value) in &fork.deposit_contract_data.storage {
            let slot_u64: u64 = (*slot).try_into().unwrap();
            assert!(
                slot_u64 >= 0x22 && slot_u64 <= 0x40,
                "Storage slot {:x} out of range",
                slot_u64
            );
        }
    }
}
