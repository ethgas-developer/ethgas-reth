//! The canonical-block oracle counts each check on which the block pending assembled fails
//! against the canonical block of the same number.
//!
//! One test, on purpose. `Metrics::default()` binds every counter to whatever recorder is global
//! at its first call in the process, so the recorder must be installed before the harness builds
//! the flashblocks state.

use std::time::{Duration, Instant};

use alloy_eips::{eip4895::Withdrawal, eip7685::Requests};
use alloy_primitives::{Address, Bytes};
use ethgas_flashblocks_node::test_harness::{FlashblockBuilder, FlashblocksBuilderTestHarness};
use ethgas_node_runner::test_utils::Account;
use eyre::{Result, eyre};
use reth_ethereum_primitives::TransactionSigned;
use reth_node_metrics::recorder::install_prometheus_recorder;

const WITHDRAWALS_ROOT: &str = "pending_withdrawals_root_mismatch";
const REQUESTS_HASH: &str = "pending_requests_hash_mismatch";
const SLOT_NUMBER: &str = "pending_slot_number_mismatch";
const TX_PREFIX: &str = "pending_tx_prefix_violation";
const CATCHUP: &str = "pending_clear_catchup";

/// The value of the counter whose name ends with `suffix`, read from reth's Prometheus recorder.
fn counter_value(suffix: &str) -> u64 {
    let rendered = install_prometheus_recorder().handle().render();
    rendered
        .lines()
        .filter_map(|line| line.split_once(' '))
        .find(|(name, _)| name.ends_with(suffix))
        .and_then(|(_, value)| value.trim().parse::<f64>().ok())
        .map_or(0, |value| value as u64)
}

/// The oracle's counters, in a fixed order.
fn oracle_counters() -> [u64; 4] {
    [WITHDRAWALS_ROOT, REQUESTS_HASH, SLOT_NUMBER, TX_PREFIX].map(counter_value)
}

/// Builds the next canonical block from `transactions` and waits until the processor has
/// reconciled pending with it. Each block here catches pending up, so the catch-up counter says
/// when the processor is done, and the oracle runs before that.
async fn canonical_block(
    test: &mut FlashblocksBuilderTestHarness,
    transactions: Vec<TransactionSigned>,
) -> Result<()> {
    let caught_up = counter_value(CATCHUP);
    test.new_canonical_block(transactions).await;
    let deadline = Instant::now() + Duration::from_secs(5);
    while counter_value(CATCHUP) == caught_up {
        if Instant::now() > deadline {
            return Err(eyre!("the processor did not reconcile the canonical block"));
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    Ok(())
}

#[tokio::test]
async fn the_oracle_counts_each_check_pending_fails_against_canonical() -> Result<()> {
    install_prometheus_recorder();
    let mut test = FlashblocksBuilderTestHarness::new().await;
    let start = oracle_counters();

    // Block 1: the flashblocks carry a withdrawal, and the latest one a requests list. The
    // canonical block has neither.
    let withdrawal =
        Withdrawal { index: 0, validator_index: 1, address: Address::with_last_byte(1), amount: 1 };
    let mut base = FlashblockBuilder::new_base(&test).build();
    base.diff.withdrawals = vec![withdrawal];
    test.send_flashblock(base).await;
    let mut second = FlashblockBuilder::new(&test, 1).build();
    second.diff.withdrawals = vec![withdrawal];
    second.diff.requests = Some(Requests::new(vec![Bytes::from_static(&[0x01, 0xaa])]));
    test.send_flashblock(second).await;
    canonical_block(&mut test, vec![]).await?;

    let [withdrawals_root, requests_hash, slot_number, tx_prefix] = oracle_counters();
    assert_eq!(withdrawals_root, start[0] + 1, "the withdrawals root differs");
    assert_eq!(requests_hash, start[1] + 1, "the requests hash differs");
    assert_eq!(slot_number, start[2], "neither block carries a slot before Amsterdam");
    assert_eq!(tx_prefix, start[3], "both blocks are empty");

    // Block 2: pending holds Alice's transfer, the canonical block Bob's.
    let after_block_1 = oracle_counters();
    test.send_flashblock(FlashblockBuilder::new_base(&test).build()).await;
    let alice = test.build_transaction_to_send_eth(Account::Alice, Account::Bob, 1);
    test.send_flashblock(FlashblockBuilder::new(&test, 1).with_transactions(vec![alice]).build())
        .await;
    let bob = test.build_transaction_to_send_eth_with_nonce(Account::Bob, Account::Alice, 1, 0);
    canonical_block(&mut test, vec![bob]).await?;

    let after_block_2 = oracle_counters();
    assert_eq!(after_block_2[3], after_block_1[3] + 1, "canonical does not start with pending");
    assert_eq!(after_block_2[..3], after_block_1[..3], "every field agrees");

    // Block 3 agrees with its canonical block on every check.
    test.send_flashblock(FlashblockBuilder::new_base(&test).build()).await;
    canonical_block(&mut test, vec![]).await?;
    assert_eq!(oracle_counters(), after_block_2, "nothing differs");

    Ok(())
}
