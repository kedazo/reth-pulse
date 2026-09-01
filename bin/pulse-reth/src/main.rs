#![allow(missing_docs)]

mod chainspec;

#[global_allocator]
static ALLOC: reth_cli_util::allocator::Allocator = reth_cli_util::allocator::new_allocator();

use chainspec::PulseChainSpecParser;
use clap::Parser;
use reth::cli::Cli;
use reth_node_builder::NodeHandle;
use reth_pulsechain_node::PulseChainNode;
use tracing::info;

fn main() {
    reth_cli_util::sigsegv_handler::install();

    // Enable backtraces unless a RUST_BACKTRACE value has already been explicitly provided.
    if std::env::var_os("RUST_BACKTRACE").is_none() {
        unsafe { std::env::set_var("RUST_BACKTRACE", "1") };
    }

    if let Err(err) = Cli::<PulseChainSpecParser>::parse().run(async move |builder, _ext_args| {
        info!(target: "pulse_reth::cli", "Launching PulseChain node");
        let NodeHandle { node: _node, node_exit_future } =
            builder.node(PulseChainNode::default()).launch_with_debug_capabilities().await?;

        node_exit_future.await
    }) {
        eprintln!("Error: {err:?}");
        std::process::exit(1);
    }
}
