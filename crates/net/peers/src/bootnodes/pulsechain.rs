//! PulseChain bootnodes
//!
//! From patch 0032: Update PulseChain bootnodes
//! Source: go-pulse/params/bootnodes.go

use crate::NodeRecord;

/// PulseChain mainnet bootnodes
///
/// These are the official PulseChain mainnet bootnodes from the go-pulse client.
/// Last updated: February 21, 2024 (patch 0032)
pub static PULSECHAIN_BOOTNODES: &[&str] = &[
    // bootnode-001-hetzner-fsn
    "enode://bdb96e7ff6607414a4be8cdc8458861e9c22a25a0c254c7bb9c9c8423912e998b59e7ba012801538480eb78cec4d6766ab0b379d0b60356de84a7cdaec988c0b@5.9.124.244:30303",
    // bootnode-002-hetzner-fsn
    "enode://d69f8d28804ab34f7d5e20ac8bd4940412602787e2c37fc3600adc60dcd5d0a52e1fe1baccbefb6e278e1ee59fcb099c45db242edeb5e0a4547ff971218a0592@148.251.54.222:30303",
    // bootnode-003-hetzner-hel
    "enode://1c9e030aa44b95b8239e1c97926787e12770c015b9dbf7a89b1178a5f4fab02462fde3489662119872dad5998e23440f78daae753d7a8f800900d871f08650a4@65.108.236.231:30303",
    // bootnode-004-hetzner-hel
    "enode://95097eaeda4118297ad0ccb6160e1c9188af7560d25b4724052e0f004a33aaddb0e468103d622c77539b692fd1d9f3c156cb76c9ea402a86e3170d6ae60092e7@135.181.212.228:30303",
    // bootnode-001 maintained by www.g4mm4.io
    "enode://da30ab2475cda64c2454b659a3ef045884c7d02b97d524d710020fdc2f37192b0aac7992bca8b7afd57474eb477e95567c8e0fe98003b779834f265304376c3c@135.181.229.180:30303",
    // bootnode-002 maintained by www.g4mm4.io
    "enode://01d93871155cbe270bc60acfebc1aa859aacce002acaac39d633aa8e7c186ee26d19a41a50d8bc094c025a546ae5e1a38dc21ead75b4e7ddf4e917988d2f7c74@46.4.224.159:30303",
    // bootnode-003 maintained by www.g4mm4.io
    "enode://96367e5e533cde68b6d3e7cc5308901fb1e4b1df51d2a0442df365fcfb8ba27a6e8bcde44b3629579da9e13d819f6059386a1e81ea4c5fd10d14599639c16214@46.4.224.160:30303",
    // bootnode-004 maintained by www.g4mm4.io
    "enode://aece632270d66ff6bf9e9528e766b5829fb3b7812d48e4934c2768c45976b5f98559ce6d5763dc16d4351b15e776b55e2b983a0c367bdbe6279cfb3242f2587e@95.217.148.233:30303",
    // bootnode-005 maintained by www.g4mm4.io
    "enode://95e1761e526d77fc732416a31c9c1795863b557ea02880101c01d14d13fdabb9312ce45c4f3037ad88002815f6826a36d86e42a1a7122f9188c64f53c4b68b1e@148.251.185.52:30303",
    // bootnode-006 maintained by www.g4mm4.io
    "enode://0ad3bc059105b0cbc1d30a330f79b4fd4ef40f37782194daa6d3412a29a69e0190dd246fc019be9157a4bf095b584ab7874beba4c71c02156f602f32ff389f00@138.201.220.52:30303",
];

/// Returns parsed PulseChain mainnet bootnodes
pub fn pulsechain_nodes() -> Vec<NodeRecord> {
    super::parse_nodes(PULSECHAIN_BOOTNODES)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pulsechain_bootnodes_parse() {
        let nodes = pulsechain_nodes();
        assert_eq!(nodes.len(), 10);

        // Verify first bootnode parses correctly
        assert_eq!(nodes[0].address.to_string(), "5.9.124.244");
        assert_eq!(nodes[0].tcp_port, 30303);
    }

    #[test]
    fn test_pulsechain_bootnodes_count() {
        assert_eq!(PULSECHAIN_BOOTNODES.len(), 10);
    }
}
