
use url::Url;

use futures_util::StreamExt;
use std::{io::Read, sync::Arc};

use crate::payload::{FlashBlock, FlashblocksPayloadV1, Metadata};
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};
use tracing::{error, info};

pub trait FlashblocksReceiver {
    fn on_flashblock_received(&self, flashblock: FlashBlock);
}

// Simplify actor messages to just handle shutdown
#[derive(Debug)]
enum ActorMessage {
    BestPayload { payload: FlashBlock },
}

pub struct FlashblocksSubscriber<Receiver> {
    flashblocks_state: Arc<Receiver>,
    ws_url: Url,
}

impl<Receiver> FlashblocksSubscriber<Receiver>
where
    Receiver: FlashblocksReceiver + Send + Sync + 'static,
{
    pub fn new(flashblocks_state: Arc<Receiver>, ws_url: Url) -> Self {
        Self { ws_url, flashblocks_state }
    }

    pub fn start(&mut self) {
        info!(
            message = "Starting Flashblocks subscription",
            url = %self.ws_url,
        );

        let ws_url = self.ws_url.clone();

        let (sender, mut mailbox) = mpsc::channel(100);

        tokio::spawn(async move {
            let mut backoff = std::time::Duration::from_secs(1);
            const MAX_BACKOFF: std::time::Duration = std::time::Duration::from_secs(10);

            loop {
                match connect_async(ws_url.as_str()).await {
                    Ok((ws_stream, _)) => {
                        info!(message = "WebSocket connection established");

                        let (_, mut read) = ws_stream.split();

                        while let Some(msg) = read.next().await {
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
                                Ok(Message::Text(_)) => {
                                    error!(
                                        "Received flashblock as plaintext, only compressed flashblocks supported. Set up websocket-proxy to use compressed flashblocks."
                                    );
                                }
                                Ok(Message::Close(_)) => {
                                    info!(message = "WebSocket connection closed by upstream");
                                    break;
                                }
                                Err(e) => {
                                    error!(
                                        message = "error receiving message",
                                        error = %e
                                    );
                                    break;
                                }
                                _ => {}
                            }
                        }
                    }
                    Err(e) => {
                        error!(
                            message = "WebSocket connection error, retrying",
                            backoff_duration = ?backoff,
                            error = %e
                        );
                        tokio::time::sleep(backoff).await;
                        backoff = std::cmp::min(backoff * 2, MAX_BACKOFF);
                        continue;
                    }
                }
            }
        });

        let flashblocks_state = self.flashblocks_state.clone();
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
}

fn try_decode_message(bytes: &[u8]) -> eyre::Result<FlashBlock> {
    let text = try_parse_message(bytes)?;

    let payload: FlashblocksPayloadV1 = match serde_json::from_str(&text) {
        Ok(m) => m,
        Err(e) => {
            return Err(eyre::eyre!("failed to parse message: {}", e));
        }
    };

    let metadata: Metadata = match serde_json::from_value(payload.metadata.clone()) {
        Ok(m) => m,
        Err(e) => {
            return Err(eyre::eyre!("failed to parse message metadata: {}", e));
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
