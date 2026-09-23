# eth_subscribe

Opens a real-time subscription over a WebSocket connection. Returns a subscription ID; events then arrive as unsolicited `eth_subscription` notifications.

| | |
|---|---|
| Flashblocks `pending` | **Yes** — flashblock subscriptions are available here |

## Parameters

| Name | Type | Required | Description |
|---|---|---|---|
| `subscriptionType` | string | yes | The event type. See the table below |
| `params` | object \| boolean | no | Filter or mode, interpreted per subscription type |

**Standard types:**

| Type | Fires with |
|---|---|
| `newHeads` | Each new confirmed block header |
| `logs` | Logs matching a filter |
| `newPendingTransactions` | Transaction hashes entering the pool |
| `syncing` | Sync status changes |

**Flashblocks types**, available when the node is connected to a flashblocks feed:

| Type | Parameter | Fires with |
|---|---|---|
| `newFlashblocks` | none | The current pending block, on every new flashblock |
| `pendingLogs` | a log filter | Logs from the latest flashblock matching the filter |
| `newFlashblockTransactions` | `true`, a log filter, or none | Transactions from the latest flashblock |

## Returns

`string` — the subscription ID, hex-encoded.

## Example

**Request**

```json
{
  "jsonrpc": "2.0",
  "method": "eth_subscribe",
  "params": ["newFlashblockTransactions", true],
  "id": 1
}
```

**Response**

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": "0x1887ec8b9589ccad00000000000532da"
}
```

## Behaviour at `pending` on this node

Subscriptions require a WebSocket connection. They cannot be served over HTTP.

`newFlashblockTransactions` takes three parameter forms:

- `true` — the full transaction plus the receipt fields flashblock execution already knows
- a log filter — the same payload, but only for transactions where a log matches
- no parameter — transaction hashes only

The full form flattens the transaction object and adds these fields, each serialized exactly as
`eth_getTransactionReceipt` serializes it:

| Field | Type | Notes |
|---|---|---|
| `logs` | array | Logs emitted by this transaction |
| `logsBloom` | `Bloom` | Bloom over `logs` |
| `gasUsed` | hex quantity | Gas this transaction alone consumed |
| `cumulativeGasUsed` | hex quantity | Gas used by the block up to and including this transaction |
| `status` | hex quantity | `0x1` or `0x0`, flattened from the EIP-658 value |
| `contractAddress` | address \| null | Set only when this transaction deployed a contract |

Because the transaction object is flattened in, the payload carries `blockHash: null` for a
pre-confirmed transaction, exactly as
[`eth_getTransactionByHash`](./eth_getTransactionByHash.md) does.

`newHeads` fires on confirmed blocks only. It does **not** fire per flashblock. To follow the
pre-confirmed head, use `newFlashblocks`.
