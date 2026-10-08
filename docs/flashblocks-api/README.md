# ETHGas Flashblocks API

The methods and subscriptions that exist only on a flashblocks-enabled node.

For the standard Ethereum methods and what each does with the `pending` tag, see
[RPC Overview](../rpc-overview.md).

---

## Methods and subscriptions

One page each, with parameters, payload shapes and worked examples.

**Methods**

| Method | Purpose |
|---|---|
| [eth_simulateV1](./eth_simulateV1.md) | Simulate call bundles against pre-confirmed state |
| [eth_sendRawTransactionSync](./eth_sendRawTransactionSync.md) | Submit a transaction and wait for pre-confirmation |
| [ethgas_inclusionPriorityFee](./ethgas_inclusionPriorityFee.md) | The builder's inclusion fee, with its source and age |

**Subscriptions** — WebSocket or IPC, through `eth_subscribe`

| Subscription | Streams |
|---|---|
| [newFlashblocks](./newFlashblocks.md) | The pending block, on every new flashblock |
| [newFlashblockTransactions](./newFlashblockTransactions.md) | Each transaction as it is sequenced |
| [pendingLogs](./pendingLogs.md) | Logs from pre-confirmed transactions, one per message |

For these methods and subscriptions used together with the standard ones, from reading the
pre-confirmed block to waiting for the seal, see [Examples](../examples/README.md).

For the standard Ethereum methods and what each does with `pending`, see
[RPC Overview](../rpc-overview.md).
