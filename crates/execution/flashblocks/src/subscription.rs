//! WebSocket subscription handling for flashblocks.

use std::{collections::HashSet, io::Read, sync::Arc, time::Duration};

use futures_util::{SinkExt, StreamExt};
use tokio::{
    sync::mpsc,
    time::{Instant, interval_at},
};
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};
use tracing::{error, info, trace, warn};
use url::Url;

use crate::{
    metrics::Metrics,
    payload::{FlashBlock, FlashblocksPayloadV1, Metadata},
    traits::FlashblocksReceiver,
};

/// Maximum size of a flashblock payload after decoding, in bytes.
const MAX_DECODED_FLASHBLOCK_BYTES: usize = 5 * 1024 * 1024;

/// The keys of each wire object this node reads. A producer that adds a key ahead of this node
/// must not stop decoding, so any other key is counted and logged, not refused.
const ENVELOPE_KEYS: &[&str] = &["payload_id", "index", "base", "diff", "metadata"];
const BASE_KEYS: &[&str] = &[
    "parent_beacon_block_root",
    "parent_hash",
    "fee_recipient",
    "prev_randao",
    "block_number",
    "gas_limit",
    "timestamp",
    "extra_data",
    "base_fee_per_gas",
    "slot_number",
];
const DIFF_KEYS: &[&str] = &[
    "state_root",
    "receipts_root",
    "logs_bloom",
    "gas_used",
    "block_hash",
    "transactions",
    "withdrawals",
    "blob_gas_used",
    "excess_blob_gas",
    "requests",
];
const METADATA_KEYS: &[&str] =
    &["block_number", "new_account_balances", "receipts", "inclusion_fee"];

/// How many distinct unknown keys the subscriber warns about; the counter counts every one.
const MAX_WARNED_UNKNOWN_KEYS: usize = 64;

// Simplify actor messages to just handle shutdown
#[derive(Debug)]
enum ActorMessage {
    BestPayload { payload: FlashBlock },
}

/// Subscribes to flashblocks via WebSocket and forwards them to the receiver.
#[derive(Debug)]
pub struct FlashblocksSubscriber<Receiver> {
    flashblocks_state: Arc<Receiver>,
    metrics: Metrics,
    ws_url: Url,
    ping_interval: Duration,
}

impl<Receiver> FlashblocksSubscriber<Receiver>
where
    Receiver: FlashblocksReceiver + Send + Sync + 'static,
{
    /// Max duration of backoff before reconnecting to upstream.
    pub const MAX_BACKOFF: Duration = Duration::from_secs(10);

    /// Creates a new flashblocks subscriber.
    pub fn new(flashblocks_state: Arc<Receiver>, ws_url: Url, ping_interval: Duration) -> Self {
        Self { ws_url, flashblocks_state, metrics: Metrics::default(), ping_interval }
    }

    /// Starts the WebSocket subscription to receive flashblocks.
    pub fn start(&mut self) {
        info!(
            message = "Starting Flashblocks subscription",
            url = %self.ws_url,
        );

        let ws_url = self.ws_url.clone();
        let ping_period = self.ping_interval;

        let (sender, mut mailbox) = mpsc::channel(100);
        let metrics = self.metrics.clone();

        tokio::spawn(async move {
            let mut backoff = Duration::from_secs(1);
            let mut warned_keys = HashSet::new();

            loop {
                match connect_async(ws_url.as_str()).await {
                    Ok((ws_stream, _)) => {
                        backoff = Duration::from_secs(1);
                        info!(message = "WebSocket connection established");

                        let mut ping_interval =
                            interval_at(Instant::now() + ping_period, ping_period);
                        let mut awaiting_pong_resp = false;

                        let (mut write, mut read) = ws_stream.split();

                        'conn: loop {
                            tokio::select! {
                                Some(msg) = read.next() => {
                                    metrics.upstream_messages.increment(1);

                                    match msg {
                                        Ok(Message::Binary(bytes)) => match try_decode_message(&bytes) {
                                            Ok((payload, shape)) => {
                                                record_wire_shape(&metrics, &mut warned_keys, &shape);
                                                let _ = sender.send(ActorMessage::BestPayload { payload: payload.clone() }).await.map_err(|e| {
                                                    error!(message = "Failed to publish message to channel", error = %e);
                                                });
                                            }
                                            Err(e) => {
                                                error!(
                                                    message = "error decoding flashblock message",
                                                    error = %e
                                                );
                                            }
                                        },
                                        Ok(Message::Text(text)) => {
                                            match try_decode_plaintext_message(&text) {
                                                Ok((payload, shape)) => {
                                                    record_wire_shape(&metrics, &mut warned_keys, &shape);
                                                    let _ = sender.send(ActorMessage::BestPayload { payload: payload.clone() }).await.map_err(|e| {
                                                        error!(message = "Failed to publish message to channel", error = %e);
                                                    });
                                                }
                                                Err(e) => {
                                                    error!(
                                                        message = "error decoding plaintext flashblock message",
                                                        error = %e
                                                    );
                                                }
                                            }
                                        }
                                        Ok(Message::Close(_)) => {
                                            info!(message = "WebSocket connection closed by upstream");
                                            break;
                                        }
                                        Ok(Message::Pong(data)) => {
                                            trace!(target: "flashblocks_rpc::subscription",
                                                ?data,
                                                "Received pong from upstream"
                                            );
                                            awaiting_pong_resp = false
                                        }
                                        Err(e) => {
                                            metrics.upstream_errors.increment(1);
                                            error!(
                                                message = "error receiving message",
                                                error = %e
                                            );
                                            break;
                                        }
                                        _ => {}
                                    }
                                },
                                _ = ping_interval.tick() => {
                                    if awaiting_pong_resp {
                                          warn!(
                                            target: "flashblocks_rpc::subscription",
                                            ?backoff,
                                            timeout = ?ping_period,
                                            "No pong response from upstream, reconnecting",
                                        );

                                        backoff = Self::sleep(&metrics, backoff).await;
                                        break 'conn;
                                    }

                                    trace!(target: "flashblocks_rpc::subscription",
                                        "Sending ping to upstream"
                                    );

                                    if let Err(error) = write.send(Message::Ping(Default::default())).await {
                                        warn!(
                                            target: "flashblocks_rpc::subscription",
                                            ?backoff,
                                            %error,
                                            "WebSocket connection lost, reconnecting",
                                        );

                                        backoff = Self::sleep(&metrics, backoff).await;
                                        break 'conn;
                                    }
                                    awaiting_pong_resp = true
                                }
                            }
                        }
                    }
                    Err(e) => {
                        error!(
                            message = "WebSocket connection error, retrying",
                            backoff_duration = ?backoff,
                            error = %e
                        );

                        backoff = Self::sleep(&metrics, backoff).await;
                    }
                }
            }
        });

        let flashblocks_state = Arc::clone(&self.flashblocks_state);
        tokio::spawn(async move {
            while let Some(message) = mailbox.recv().await {
                match message {
                    ActorMessage::BestPayload { payload } => {
                        flashblocks_state.on_flashblock_received(payload);
                    }
                }
            }
        });
    }

    /// Sleeps for given backoff duration. Returns incremented backoff duration, capped at
    /// [`Self::MAX_BACKOFF`].
    async fn sleep(metrics: &Metrics, backoff: Duration) -> Duration {
        metrics.reconnect_attempts.increment(1);
        tokio::time::sleep(backoff).await;
        std::cmp::min(backoff * 2, Self::MAX_BACKOFF)
    }
}

/// What a flashblock carries beyond, or short of, what this node reads.
#[derive(Debug, Default, PartialEq, Eq)]
struct WireShape {
    /// Keys this node does not read, as `object.key`.
    unknown_keys: Vec<String>,
    /// The flashblock has a base payload, and it carries no slot number.
    base_without_slot_number: bool,
    /// The diff carries no requests list.
    diff_without_requests: bool,
}

impl WireShape {
    fn of(payload: &serde_json::Value) -> Self {
        let mut unknown_keys = Vec::new();
        for (object, value, known) in [
            ("envelope", Some(payload), ENVELOPE_KEYS),
            ("base", payload.get("base"), BASE_KEYS),
            ("diff", payload.get("diff"), DIFF_KEYS),
            ("metadata", payload.get("metadata"), METADATA_KEYS),
        ] {
            let Some(map) = value.and_then(serde_json::Value::as_object) else { continue };
            unknown_keys.extend(
                map.keys()
                    .filter(|key| !known.contains(&key.as_str()))
                    .map(|key| format!("{object}.{key}")),
            );
        }

        // A null decodes as an absent value, so it counts as one here too.
        let present =
            |value: Option<&serde_json::Value>| value.is_some_and(|value| !value.is_null());
        let base = payload.get("base").filter(|base| !base.is_null());
        Self {
            unknown_keys,
            base_without_slot_number: base.is_some_and(|base| !present(base.get("slot_number"))),
            diff_without_requests: !present(
                payload.get("diff").and_then(|diff| diff.get("requests")),
            ),
        }
    }
}

/// Counts what `shape` reports, and warns once for each unknown key, for the first
/// [`MAX_WARNED_UNKNOWN_KEYS`] distinct keys.
fn record_wire_shape(metrics: &Metrics, warned_keys: &mut HashSet<String>, shape: &WireShape) {
    for key in &shape.unknown_keys {
        metrics.unknown_wire_fields.increment(1);
        if warned_keys.len() < MAX_WARNED_UNKNOWN_KEYS && warned_keys.insert(key.clone()) {
            warn!(message = "flashblock carries a key this node does not read", key = %key);
        }
    }
    if shape.base_without_slot_number {
        metrics.wire_slot_number_missing.increment(1);
    }
    if shape.diff_without_requests {
        metrics.wire_requests_missing.increment(1);
    }
}

fn try_decode_message(bytes: &[u8]) -> eyre::Result<(FlashBlock, WireShape)> {
    let text = try_parse_message(bytes)?;
    parse_flashblock_json(&text)
}

fn try_decode_plaintext_message(text: &str) -> eyre::Result<(FlashBlock, WireShape)> {
    ensure_within_size_limit(text.len())?;
    parse_flashblock_json(text)
}

fn parse_flashblock_json(text: &str) -> eyre::Result<(FlashBlock, WireShape)> {
    let value: serde_json::Value = match serde_json::from_str(text) {
        Ok(value) => value,
        Err(e) => {
            return Err(eyre::eyre!("failed to parse flashblock JSON: {}", e));
        }
    };
    let shape = WireShape::of(&value);

    let payload: FlashblocksPayloadV1 = match serde_json::from_value(value) {
        Ok(m) => m,
        Err(e) => {
            return Err(eyre::eyre!("failed to parse flashblock JSON: {}", e));
        }
    };

    let metadata: Metadata = match serde_json::from_value(payload.metadata.clone()) {
        Ok(m) => m,
        Err(e) => {
            return Err(eyre::eyre!("failed to parse flashblock metadata: {}", e));
        }
    };

    let flashblock = FlashBlock {
        payload_id: payload.payload_id,
        index: payload.index,
        base: payload.base,
        diff: payload.diff,
        metadata,
    };
    Ok((flashblock, shape))
}

fn try_parse_message(bytes: &[u8]) -> eyre::Result<String> {
    if let Ok(text) = std::str::from_utf8(bytes) &&
        text.trim_start().starts_with('{')
    {
        ensure_within_size_limit(bytes.len())?;
        return Ok(text.to_owned());
    }

    let mut decompressor =
        brotli::Decompressor::new(bytes, 4096).take(MAX_DECODED_FLASHBLOCK_BYTES as u64 + 1);
    let mut decompressed = Vec::new();
    decompressor.read_to_end(&mut decompressed)?;
    ensure_within_size_limit(decompressed.len())?;

    let text = String::from_utf8(decompressed)?;
    Ok(text)
}

fn ensure_within_size_limit(len: usize) -> eyre::Result<()> {
    eyre::ensure!(
        len <= MAX_DECODED_FLASHBLOCK_BYTES,
        "flashblock payload too large: {len} bytes, max {MAX_DECODED_FLASHBLOCK_BYTES}"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use alloy_eips::eip7685::Requests;
    use alloy_primitives::{B256, Bytes, U256};

    use super::*;
    use crate::payload::{ExecutionPayloadBaseV1, ExecutionPayloadFlashblockDeltaV1, InclusionFee};

    fn brotli_compress(bytes: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut writer = brotli::CompressorWriter::new(&mut out, 4096, 11, 22);
        writer.write_all(bytes).expect("compress");
        drop(writer);
        out
    }

    #[test]
    fn decompression_bomb_is_rejected() {
        // ~64 MiB of highly compressible input squeezes into a few KB on the wire.
        let bomb = brotli_compress(&vec![b'a'; 64 * 1024 * 1024]);
        assert!(bomb.len() < 64 * 1024, "bomb should be small on the wire, got {}", bomb.len());

        // Match on the result rather than unwrapping it: if this ever regresses, the decoded
        // payload is 64 MiB and a panic message carrying it would swamp the test output.
        match try_parse_message(&bomb) {
            Err(err) => assert!(err.to_string().contains("too large"), "unexpected error: {err}"),
            Ok(decoded) => panic!("bomb accepted, decoded {} bytes", decoded.len()),
        }
    }

    #[test]
    fn brotli_payload_within_limit_is_accepted() {
        let json = r#"{"payload_id":"0x0000000000000000"}"#;
        let parsed = try_parse_message(&brotli_compress(json.as_bytes())).expect("accepted");
        assert_eq!(parsed, json);
    }

    #[test]
    fn plaintext_payload_is_accepted() {
        let json = r#"{"payload_id":"0x0000000000000000"}"#;
        assert_eq!(try_parse_message(json.as_bytes()).expect("accepted"), json);
    }

    #[test]
    fn oversized_plaintext_is_rejected() {
        let mut json = String::from("{\"a\":\"");
        json.push_str(&"x".repeat(MAX_DECODED_FLASHBLOCK_BYTES));
        json.push_str("\"}");

        let err = try_parse_message(json.as_bytes()).expect_err("must reject");
        assert!(err.to_string().contains("too large"), "unexpected error: {err}");
    }

    /// A flashblock whose metadata is `metadata`, as the producer sends it.
    fn flashblock_json(metadata: serde_json::Value) -> String {
        serde_json::to_string(&FlashblocksPayloadV1 { metadata, ..Default::default() })
            .expect("a payload serializes")
    }

    fn metadata_with_inclusion_fee(inclusion_fee: serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "block_number": 1,
            "new_account_balances": {},
            "receipts": {},
            "inclusion_fee": inclusion_fee,
        })
    }

    #[test]
    fn a_malformed_inclusion_fee_rejects_the_whole_flashblock() {
        let text = flashblock_json(metadata_with_inclusion_fee(
            serde_json::json!({"priority_fee": "not a quantity"}),
        ));

        let err = parse_flashblock_json(&text).expect_err("must reject");
        assert!(err.to_string().contains("metadata"), "unexpected error: {err}");
    }

    #[test]
    fn a_null_inclusion_fee_decodes_as_none() {
        let text = flashblock_json(metadata_with_inclusion_fee(serde_json::Value::Null));

        let (parsed, _) = parse_flashblock_json(&text).expect("accepted");
        assert_eq!(parsed.metadata.inclusion_fee, None);
        assert_eq!(parsed.metadata.block_number, 1);
    }

    #[test]
    fn an_unknown_key_in_the_inclusion_fee_is_ignored() {
        let text = flashblock_json(metadata_with_inclusion_fee(
            serde_json::json!({"priority_fee": "0x10", "tier": "high"}),
        ));

        let (parsed, _) = parse_flashblock_json(&text).expect("accepted");
        assert_eq!(
            parsed.metadata.inclusion_fee,
            Some(InclusionFee { priority_fee: U256::from(16) })
        );
    }

    /// A base flashblock as a producer that predates the Amsterdam fields sends it.
    fn pre_amsterdam_flashblock() -> serde_json::Value {
        let mut flashblock = serde_json::to_value(FlashblocksPayloadV1 {
            base: Some(ExecutionPayloadBaseV1::default()),
            metadata: serde_json::json!({
                "block_number": 1,
                "new_account_balances": {},
                "receipts": {},
            }),
            ..Default::default()
        })
        .expect("a payload serializes");
        flashblock["base"].as_object_mut().expect("an object").remove("slot_number");
        flashblock["diff"].as_object_mut().expect("an object").remove("requests");
        flashblock
    }

    #[test]
    fn a_producer_without_the_amsterdam_fields_decodes_them_as_absent() {
        let text = pre_amsterdam_flashblock().to_string();

        let (parsed, _) = parse_flashblock_json(&text).expect("accepted");
        assert_eq!(parsed.base.expect("a base payload").slot_number, None);
        assert_eq!(parsed.diff.requests, None);
    }

    #[test]
    fn the_amsterdam_fields_decode_from_the_wire_encoding() {
        let mut flashblock = pre_amsterdam_flashblock();
        flashblock["base"]["slot_number"] = serde_json::json!("0x1a2b");
        flashblock["diff"]["requests"] = serde_json::json!(["0x00aa", "0x01bb"]);

        let (parsed, _) = parse_flashblock_json(&flashblock.to_string()).expect("accepted");
        assert_eq!(parsed.base.expect("a base payload").slot_number, Some(0x1a2b));
        assert_eq!(
            parsed.diff.requests,
            Some(Requests::new(vec![
                Bytes::from_static(&[0x00, 0xaa]),
                Bytes::from_static(&[0x01, 0xbb]),
            ]))
        );
    }

    #[test]
    fn every_key_the_node_reads_is_known() {
        let payload = FlashblocksPayloadV1 {
            base: Some(ExecutionPayloadBaseV1 { slot_number: Some(1), ..Default::default() }),
            diff: ExecutionPayloadFlashblockDeltaV1 {
                requests: Some(Requests::default()),
                ..Default::default()
            },
            metadata: serde_json::to_value(Metadata {
                inclusion_fee: Some(InclusionFee { priority_fee: U256::from(1) }),
                ..Default::default()
            })
            .expect("metadata serializes"),
            ..Default::default()
        };

        let shape = WireShape::of(&serde_json::to_value(&payload).expect("a payload serializes"));
        assert_eq!(shape, WireShape::default());
    }

    #[test]
    fn an_unknown_key_at_each_level_is_reported_and_the_flashblock_still_decodes() {
        let mut flashblock = pre_amsterdam_flashblock();
        flashblock["builder_version"] = serde_json::json!("2");
        flashblock["base"]["block_access_list_hash"] = serde_json::json!(B256::ZERO);
        flashblock["diff"]["state_root_v2"] = serde_json::json!(B256::ZERO);
        flashblock["metadata"]["tier"] = serde_json::json!("high");

        let (parsed, shape) = parse_flashblock_json(&flashblock.to_string()).expect("accepted");
        assert_eq!(parsed.metadata.block_number, 1);
        let mut unknown_keys = shape.unknown_keys;
        unknown_keys.sort();
        assert_eq!(
            unknown_keys,
            [
                "base.block_access_list_hash",
                "diff.state_root_v2",
                "envelope.builder_version",
                "metadata.tier"
            ]
        );
    }

    /// Two flashblocks of one block, written by hand in the producer's wire format: an
    /// Amsterdam producer's, and one from a producer that predates the Amsterdam fields.
    const AMSTERDAM_FIXTURE: &str = include_str!("../tests/assets/flashblock_amsterdam.json");
    const PRAGUE_FIXTURE: &str = include_str!("../tests/assets/flashblock_prague.json");

    fn fixture_messages(fixture: &str) -> Vec<serde_json::Value> {
        serde_json::from_str(fixture).expect("a fixture is a JSON list of messages")
    }

    /// The decoder reads every key the producer sends and loses nothing: encoding what it
    /// decoded gives the message back.
    #[test]
    fn the_amsterdam_fixture_round_trips() {
        for message in fixture_messages(AMSTERDAM_FIXTURE) {
            let (flashblock, shape) =
                parse_flashblock_json(&message.to_string()).expect("accepted");
            assert_eq!(shape, WireShape::default());

            let encoded = serde_json::to_value(FlashblocksPayloadV1 {
                payload_id: flashblock.payload_id,
                index: flashblock.index,
                base: flashblock.base,
                diff: flashblock.diff,
                metadata: serde_json::to_value(flashblock.metadata).expect("metadata serializes"),
            })
            .expect("a payload serializes");
            assert_eq!(encoded, message);
        }
    }

    #[test]
    fn the_prague_fixture_decodes_without_the_amsterdam_fields() {
        for message in fixture_messages(PRAGUE_FIXTURE) {
            let (flashblock, shape) =
                parse_flashblock_json(&message.to_string()).expect("accepted");
            assert_eq!(flashblock.base.as_ref().and_then(|base| base.slot_number), None);
            assert_eq!(flashblock.diff.requests, None);
            assert_eq!(shape.base_without_slot_number, flashblock.base.is_some());
            assert!(shape.diff_without_requests);
            assert!(shape.unknown_keys.is_empty());
        }
    }

    #[test]
    fn a_field_the_node_does_not_read_is_reported_not_refused() {
        let mut message = fixture_messages(AMSTERDAM_FIXTURE).remove(0);
        message["diff"]["block_access_list_hash"] = serde_json::json!(B256::ZERO);

        let (flashblock, shape) = parse_flashblock_json(&message.to_string()).expect("accepted");
        assert_eq!(flashblock.base.expect("a base payload").slot_number, Some(0x1a2b));
        assert_eq!(shape.unknown_keys, ["diff.block_access_list_hash"]);
    }

    #[test]
    fn the_shape_says_which_amsterdam_fields_a_flashblock_lacks() {
        let (_, old) = parse_flashblock_json(&pre_amsterdam_flashblock().to_string()).unwrap();
        assert!(old.base_without_slot_number);
        assert!(old.diff_without_requests);

        let mut nulls = pre_amsterdam_flashblock();
        nulls["base"]["slot_number"] = serde_json::Value::Null;
        nulls["diff"]["requests"] = serde_json::Value::Null;
        let (_, nulls) = parse_flashblock_json(&nulls.to_string()).unwrap();
        assert!(nulls.base_without_slot_number);
        assert!(nulls.diff_without_requests);

        let mut diff_only = pre_amsterdam_flashblock();
        diff_only.as_object_mut().expect("an object").remove("base");
        diff_only["diff"]["requests"] = serde_json::json!([]);
        let (_, diff_only) = parse_flashblock_json(&diff_only.to_string()).unwrap();
        assert!(!diff_only.base_without_slot_number, "a diff-only flashblock has no slot to miss");
        assert!(!diff_only.diff_without_requests);
    }
}
