# pulse-reth / reth-pulse

This repository contains a modded version of 'reth' client with PulseChain
support. I am unsure if the changes made here are breaking the Ethereum mainnet
support, this repository is solely made to have a 'reth' client working on
PulseChain mainnet.

Original README.md can be found in [README.orig.md](README.orig.md)

## Status

Currently I am doing excessive testing of the modifications, so far it could
sync and execute blocks from 0 till and after PulseChain PrimOrdial block
(17233000). I have tested using [Prysm-Pulse](https://gitlab.com/pulsechaincom/prysm-pulse) consensus client.

## Build & install

Make sure you have latest Rust installed [link](https://rust-lang.org/tools/install/).

To compile in release mode you may use the following command
```sh
git clone https://github.com/kedazo/reth-pulse.git
cd reth-pulse
cargo build --package pulse-reth --release --features "jemalloc,asm-keccak"
```

And it compiles the ./target/release/pulse-reth binary.

You may start your PulseChain reth node using the following command (add/apply
your own arguments):
```sh
./target/release/pulse-reth node \
    --chain=pulsechain \
    --full \
    --instance=1 \
    --datadir=./datadir \
    --authrpc.addr=0.0.0.0 \
    --authrpc.port=32651 \
    --authrpc.jwtsecret=./datadir/jwt.txt \
    --rpc.gascap=32000000000 \
    --rpc.txfeecap=0
```

Please note I am unsure about the required gascap/txfeecap values just passed
some high ones, Pulsechain requires bigger values than the default Ethereum
ones.
