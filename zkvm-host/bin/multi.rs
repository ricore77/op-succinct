use std::fs;

use anyhow::Result;
use clap::Parser;
use client_utils::precompiles::PRECOMPILE_HOOK_FD;
use host_utils::{
    fetcher::{ChainMode, SP1KonaDataFetcher},
    get_sp1_stdin, ProgramType,
};
use kona_host::start_server_and_native_client;
use sp1_sdk::{utils, ExecutionReport, ProverClient};
use zkvm_host::{precompile_hook, ExecutionStats};

pub const MULTI_BLOCK_ELF: &[u8] = include_bytes!("../../elf/validity-client-elf");

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Start L2 block number.
    #[arg(short, long)]
    start: u64,

    /// End L2 block number.
    #[arg(short, long)]
    end: u64,

    /// Skip running native execution.
    #[arg(short, long)]
    use_cache: bool,

    /// Generate proof.
    #[arg(short, long)]
    prove: bool,
}

/// Based on the stats flag, print out simple or detailed statistics.
async fn print_stats(data_fetcher: &SP1KonaDataFetcher, args: &Args, report: &ExecutionReport) {
    // Get the total instruction count for execution across all blocks.
    let block_execution_instruction_count: u64 =
        *report.cycle_tracker.get("block-execution").unwrap();

    let nb_blocks = args.end - args.start + 1;

    // Fetch the number of transactions in the blocks from the L2 RPC.
    let block_data_range = data_fetcher
        .get_block_data_range(ChainMode::L2, args.start, args.end)
        .await
        .expect("Failed to fetch block data range.");

    let nb_transactions = block_data_range.iter().map(|b| b.transaction_count).sum();
    let total_gas_used = block_data_range.iter().map(|b| b.gas_used).sum();

    println!(
        "{}",
        ExecutionStats {
            total_instruction_count: report.total_instruction_count(),
            block_execution_instruction_count,
            nb_blocks,
            nb_transactions,
            total_gas_used,
        }
    );
}

/// Execute the Kona program for a single block.
#[tokio::main]
async fn main() -> Result<()> {
    dotenv::dotenv().ok();
    utils::setup_logger();
    let args = Args::parse();

    let data_fetcher = SP1KonaDataFetcher {
        ..Default::default()
    };

    let host_cli = data_fetcher
        .get_host_cli_args(args.start, args.end, ProgramType::Multi)
        .await?;

    let data_dir = host_cli
        .data_dir
        .clone()
        .expect("Data directory is not set.");

    // By default, re-run the native execution unless the user passes `--use-cache`.
    if !args.use_cache {
        // Overwrite existing data directory.
        fs::create_dir_all(&data_dir).unwrap();

        // Start the server and native client.
        start_server_and_native_client(host_cli.clone())
            .await
            .unwrap();
    }

    // Get the stdin for the block.
    let sp1_stdin = get_sp1_stdin(&host_cli)?;

    let prover = ProverClient::new();

    if args.prove {
        // If the prove flag is set, generate a proof.
        let (pk, _) = prover.setup(MULTI_BLOCK_ELF);
        let proof = prover.prove(&pk, sp1_stdin).run().unwrap();

        // Save the proof to data/proofs.
        proof
            .save(format!("data/proofs/{}-{}.bin", args.start, args.end))
            .expect("saving proof failed");
    } else {
        // TODO: Remove this precompile hook once we merge the BN and BLS precompiles.
        let (_, report) = prover
            .execute(MULTI_BLOCK_ELF, sp1_stdin)
            .with_hook(PRECOMPILE_HOOK_FD, precompile_hook)
            .run()
            .unwrap();

        print_stats(&data_fetcher, &args, &report).await;
    }

    Ok(())
}