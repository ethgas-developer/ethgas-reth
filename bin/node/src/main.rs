//! Binary entrypoint for the Ethgas node.

pub mod cli;

use ethgas_flashblocks_node::FlashblocksExtension;
use ethgas_node_runner::EthgasNodeRunner;
use ethgas_reth_flashblocks::FlashblocksConfig;
use reth_ethereum_cli::{Cli, chainspec::EthereumChainSpecParser};

type NodeCli = Cli<EthereumChainSpecParser, cli::Args>;

#[global_allocator]
static ALLOC: reth_cli_util::allocator::Allocator = reth_cli_util::allocator::new_allocator();

// Required for "override_allocator_on_supported_platforms".
#[cfg(all(feature = "jemalloc", unix))]
use reth_cli_util::allocator::tikv_jemalloc_sys as _;

#[cfg(all(feature = "jemalloc-prof", unix))]
#[unsafe(export_name = "malloc_conf")]
static MALLOC_CONF: &[u8] = b"prof:true,prof_active:true,lg_prof_sample:19\0";

fn main() {
    ethgas_cli_utils::init_reth!();

    let cli = ethgas_cli_utils::parse_cli!(NodeCli);

    cli.run(|builder, args| async move {
        let mut runner = EthgasNodeRunner::new();

        let flashblocks_config: Option<FlashblocksConfig> = (&args).into();
        runner.install_ext::<FlashblocksExtension>(flashblocks_config);

        runner.run(builder).await
    })
    .unwrap();
}
