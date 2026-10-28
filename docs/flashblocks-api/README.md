# ETHGas Flashblocks API

The methods and subscriptions that a flashblocks-enabled node adds, and the standard methods that it
runs on pre-confirmed state.

For the standard Ethereum methods and what each does with the `pending` tag, see
[RPC Overview](../rpc-overview.md).

## Methods and subscriptions

One page each, with parameters, payload shapes and worked examples.

**Methods**

| Method | Purpose |
|---|---|
| [eth_simulateV1](./eth_simulateV1.md) | Simulate call bundles against pre-confirmed state |
| [eth_sendRawTransactionSync](./eth_sendRawTransactionSync.md) | Submit a transaction and wait for pre-confirmation |
| [ethgas_inclusionPriorityFee](./ethgas_inclusionPriorityFee.md) | The builder's inclusion fee, with its source and age |

**Subscriptions**, over WebSocket or IPC, through `eth_subscribe`

| Subscription | Streams |
|---|---|
| [newFlashblocks](./newFlashblocks.md) | The pre-confirmed block, on every new flashblock |
| [newFlashblockTransactions](./newFlashblockTransactions.md) | Each transaction as it is sequenced |
| [pendingLogs](./pendingLogs.md) | Logs from pre-confirmed transactions, one per message |
| [newFlashblockBatches](./newFlashblockBatches.md) | Each flashblock's transactions and logs, in one message |

For these methods and subscriptions used together with the standard ones, from reading the
pre-confirmed block to waiting for the seal, see [Examples](../examples/README.md).
