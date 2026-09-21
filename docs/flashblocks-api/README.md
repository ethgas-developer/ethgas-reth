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

**Subscriptions** — WebSocket only, through `eth_subscribe`

| Subscription | Streams |
|---|---|
| [newFlashblocks](./newFlashblocks.md) | The pending block, on every new flashblock |
| [newFlashblockTransactions](./newFlashblockTransactions.md) | Each transaction as it is sequenced |
| [pendingLogs](./pendingLogs.md) | Logs from pre-confirmed transactions, one per message |

For the standard Ethereum methods and what each does with `pending`, see
[RPC Overview](../rpc-overview.md).
