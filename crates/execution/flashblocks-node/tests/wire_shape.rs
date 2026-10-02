//! The subscriber counts the keys of the producer's flashblocks that this node does not read, and
//! the flashblocks that lack the Amsterdam fields, and still delivers every flashblock.
//!
//! One test, on purpose. `Metrics::default()` binds every counter to whatever recorder is global
//! at its first call in the process, so the recorder must be installed before the subscriber is
//! built.

use std::{sync::Arc, time::Duration};

use ethgas_reth_flashblocks::{
    FlashblocksReceiver, FlashblocksSubscriber,
    payload::{ExecutionPayloadBaseV1, FlashBlock, FlashblocksPayloadV1},
};
use eyre::{Result, eyre};
use futures_util::SinkExt;
use reth_node_metrics::recorder::install_prometheus_recorder;
use serde_json::{Value, json};
use tokio::{net::TcpListener, sync::mpsc, time::timeout};
use tokio_tungstenite::{accept_async, tungstenite::Message};

const UNKNOWN_KEYS: &str = "unknown_wire_fields";
const SLOT_NUMBER_MISSING: &str = "wire_slot_number_missing";
const REQUESTS_MISSING: &str = "wire_requests_missing";

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

fn counters() -> [u64; 3] {
    [UNKNOWN_KEYS, SLOT_NUMBER_MISSING, REQUESTS_MISSING].map(counter_value)
}

/// Hands every delivered flashblock to the test.
struct Collector(mpsc::UnboundedSender<FlashBlock>);

impl FlashblocksReceiver for Collector {
    fn on_flashblock_received(&self, flashblock: FlashBlock) {
        let _ = self.0.send(flashblock);
    }
}

/// A base flashblock of block 1 as a producer that predates the Amsterdam fields sends it.
fn pre_amsterdam_flashblock() -> Value {
    let mut flashblock = serde_json::to_value(FlashblocksPayloadV1 {
        base: Some(ExecutionPayloadBaseV1 { block_number: 1, ..Default::default() }),
        metadata: json!({ "block_number": 1, "new_account_balances": {}, "receipts": {} }),
        ..Default::default()
    })
    .expect("a payload serializes");
    flashblock["base"].as_object_mut().expect("an object").remove("slot_number");
    flashblock["diff"].as_object_mut().expect("an object").remove("requests");
    flashblock
}

#[tokio::test]
async fn the_subscriber_counts_unknown_keys_and_missing_amsterdam_fields() -> Result<()> {
    install_prometheus_recorder();
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let (delivered, mut received) = mpsc::unbounded_channel();
    let mut subscriber = FlashblocksSubscriber::new(
        Arc::new(Collector(delivered)),
        format!("ws://{}", listener.local_addr()?).parse()?,
        Duration::from_secs(60),
    );
    subscriber.start();
    let (stream, _) = listener.accept().await?;
    let mut producer = accept_async(stream).await?;

    let mut send_and_receive = async |flashblock: Value| -> Result<FlashBlock> {
        producer.send(Message::Text(flashblock.to_string().into())).await?;
        timeout(Duration::from_secs(5), received.recv())
            .await?
            .ok_or_else(|| eyre!("the subscriber stopped delivering"))
    };

    // An old producer's base flashblock, with one key this node does not read.
    let start = counters();
    let mut old = pre_amsterdam_flashblock();
    old["diff"]["future_field"] = json!(1);
    send_and_receive(old).await?;
    assert_eq!(counters(), [start[0] + 1, start[1] + 1, start[2] + 1]);

    // A diff-only flashblock with the requests list: no base payload, so no slot to miss.
    let after_old = counters();
    let mut diff_only = pre_amsterdam_flashblock();
    diff_only.as_object_mut().expect("an object").remove("base");
    diff_only["index"] = json!(1);
    diff_only["diff"]["requests"] = json!([]);
    send_and_receive(diff_only).await?;
    assert_eq!(counters(), after_old);

    // An Amsterdam producer's base flashblock carries both fields and nothing else.
    let mut amsterdam = pre_amsterdam_flashblock();
    amsterdam["base"]["slot_number"] = json!("0x1a2b");
    amsterdam["diff"]["requests"] = json!([]);
    let delivered = send_and_receive(amsterdam).await?;
    assert_eq!(delivered.base.expect("a base payload").slot_number, Some(0x1a2b));
    assert_eq!(counters(), after_old);

    Ok(())
}
