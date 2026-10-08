# alloy 2 (Rust)

One of the [Examples](./README.md). The program reads at `pending`, sends 1 wei and waits for its
pre-confirmation, then prints three stream messages. It takes `RPC`, `WS`, `PRIVATE_KEY` and `TO`
from the environment; see [Before you start](./README.md#before-you-start).

Tested with alloy 2.5.0 and Rust 1.95.0. The program is the `src/main.rs` of a binary crate with
these dependencies:

```toml
[dependencies]
alloy-network = "=2.5.0"
alloy-primitives = "=1.7.3"
alloy-provider = { version = "=2.5.0", features = ["reqwest", "ws", "pubsub"] }
alloy-rpc-types-eth = "=2.5.0"
alloy-signer-local = "=2.5.0"
futures = "0.3"
tokio = { version = "1", features = ["rt", "macros"] }
```

`send_transaction_sync` calls `eth_sendRawTransactionSync`. alloy's default fee estimator reads
`eth_feeHistory`, which does not carry the builder's price, so the program sets the priority fee
from `eth_maxPriorityFeePerGas`. The pending block deserializes as an ordinary `Block`.

```rust
use std::env;

use alloy_network::{EthereumWallet, TransactionBuilder};
use alloy_primitives::{B256, U256};
use alloy_provider::{Provider, ProviderBuilder, WsConnect};
use alloy_rpc_types_eth::{Block, BlockNumberOrTag, TransactionRequest};
use alloy_signer_local::PrivateKeySigner;
use futures::StreamExt;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let signer: PrivateKeySigner = env::var("PRIVATE_KEY")?.parse()?;
    let me = signer.address();
    let provider = ProviderBuilder::new()
        .wallet(EthereumWallet::from(signer))
        .connect_http(env::var("RPC")?.parse()?);

    // Read at pending
    let block = provider.get_block_by_number(BlockNumberOrTag::Pending).await?.ok_or("no block")?;
    let nonce = provider.get_transaction_count(me).pending().await?;
    let count = block.transactions.len();
    println!("pending block {} with {count} transactions; nonce {nonce}", block.header.number);

    // Send, and wait for the pre-confirmation. alloy's default fee estimator reads
    // eth_feeHistory, so the builder's inclusion fee is set here.
    let tip = provider.get_max_priority_fee_per_gas().await?;
    let base_fee = u128::from(block.header.base_fee_per_gas.ok_or("no base fee")?);
    let tx = TransactionRequest::default()
        .with_to(env::var("TO")?.parse()?)
        .with_value(U256::from(1))
        .with_max_priority_fee_per_gas(tip)
        .with_max_fee_per_gas(2 * base_fee + tip);
    let receipt = provider.send_transaction_sync(tx).await?;
    let state = if receipt.block_hash == Some(B256::ZERO) { "pre-confirmed" } else { "sealed" };
    println!("{state} in block {}", receipt.block_number.ok_or("no block number")?);

    // Stream: one message per flashblock
    let ws = ProviderBuilder::new().connect_ws(WsConnect::new(env::var("WS")?)).await?;
    let subscription = ws.subscribe::<_, Block>(("newFlashblocks",)).await?;
    let mut blocks = subscription.into_stream().take(3);
    while let Some(block) = blocks.next().await {
        let count = block.transactions.len();
        println!("block {} now has {count} transactions", block.header.number);
    }
    Ok(())
}
```

Its output on the test network:

```text
pending block 125 with 305 transactions; nonce 8
pre-confirmed in block 126
block 126 now has 152 transactions
block 126 now has 179 transactions
block 126 now has 203 transactions
```

What the library does with this node's answers elsewhere is on
[Client libraries](../client-libraries.md).
