//! WebSocket subscription handling for flashblocks.

use std::{io::Read, sync::Arc, time::Duration};

use futures_util::{SinkExt, StreamExt};
use tokio::{sync::mpsc, time::interval};
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};
use tracing::{error, info, trace, warn};
use url::Url;

use crate::{
    metrics::Metrics,
    payload::{FlashBlock, FlashblocksPayloadV1, Metadata},
    traits::FlashblocksReceiver,
};

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
}

impl<Receiver> FlashblocksSubscriber<Receiver>
where
    Receiver: FlashblocksReceiver + Send + Sync + 'static,
{
    /// Interval of liveness check of upstream, in milliseconds.
    pub const PING_INTERVAL_MS: u64 = 500;

    /// Max duration of backoff before reconnecting to upstream.
    pub const MAX_BACKOFF: Duration = Duration::from_secs(10);

    /// Creates a new flashblocks subscriber.
    pub fn new(flashblocks_state: Arc<Receiver>, ws_url: Url) -> Self {
        Self { ws_url, flashblocks_state, metrics: Metrics::default() }
    }

    /// Starts the WebSocket subscription to receive flashblocks.
    pub fn start(&mut self) {
        info!(
            message = "Starting Flashblocks subscription",
            url = %self.ws_url,
        );

        let ws_url = self.ws_url.clone();

        let (sender, mut mailbox) = mpsc::channel(100);
        let metrics = self.metrics.clone();

        tokio::spawn(async move {
            let mut backoff = Duration::from_secs(1);

            loop {
                match connect_async(ws_url.as_str()).await {
                    Ok((ws_stream, _)) => {
                        backoff = Duration::from_secs(1);
                        info!(message = "WebSocket connection established");

                        let mut ping_interval =
                            interval(Duration::from_millis(Self::PING_INTERVAL_MS));
                        let mut awaiting_pong_resp = false;

                        let (mut write, mut read) = ws_stream.split();

                        'conn: loop {
                            tokio::select! {
                                Some(msg) = read.next() => {
                                    metrics.upstream_messages.increment(1);

                                    match msg {
                                        Ok(Message::Binary(bytes)) => match try_decode_message(&bytes) {
                                            Ok(payload) => {
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
                                                Ok(payload) => {
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
                                            timeout_ms = Self::PING_INTERVAL_MS,
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
                        continue;
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

fn try_decode_message(bytes: &[u8]) -> eyre::Result<FlashBlock> {
    let text = try_parse_message(bytes)?;
    parse_flashblock_json(&text)
}

fn try_decode_plaintext_message(text: &str) -> eyre::Result<FlashBlock> {
    parse_flashblock_json(text)
}

fn parse_flashblock_json(text: &str) -> eyre::Result<FlashBlock> {
    let payload: FlashblocksPayloadV1 = match serde_json::from_str(text) {
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

    Ok(FlashBlock {
        payload_id: payload.payload_id,
        index: payload.index,
        base: payload.base,
        diff: payload.diff,
        metadata,
    })
}

fn try_parse_message(bytes: &[u8]) -> eyre::Result<String> {
    if let Ok(text) = String::from_utf8(bytes.to_vec()) {
        if text.trim_start().starts_with("{") {
            return Ok(text);
        }
    }

    let mut decompressor = brotli::Decompressor::new(bytes, 4096);
    let mut decompressed = Vec::new();
    decompressor.read_to_end(&mut decompressed)?;

    let text = String::from_utf8(decompressed)?;
    Ok(text)
}
