
# pulse-reth / reth-pulse

PulseChain reth v2.5.1

This repository contains a modded version of 'reth' client with PulseChain
support. I am unsure if the changes made here are breaking the Ethereum mainnet
support, this repository is solely made to have a 'reth' client working on
PulseChain mainnet.

This versions supports the v2/rocksdb storage.

Note: to start and build you may use the following commands:
```cargo build --package pulse-reth --release --features "jemalloc,asm-keccak"```

Then to start:
```./target/release/pulse-reth node --full # your args... etc```

Original README.md can be found in [README.orig.md](README.orig.md)

