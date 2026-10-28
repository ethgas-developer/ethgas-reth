# eth_subscribe

Opens a real-time subscription over a WebSocket or IPC connection. Returns a subscription ID; events then arrive as unsolicited `eth_subscription` notifications.

| | |
|---|---|
| Transport | WebSocket or IPC. Over HTTP the call fails with `-32603` |

## Parameters

| Name | Type | Required | Description |
|---|---|---|---|
| `subscriptionType` | string | yes | The event type. See the tables below |
| `params` | object \| boolean | no | Filter or mode, interpreted per subscription type |

**Standard types**, as on reth:

| Type | Fires with |
|---|---|
| `newHeads` | Each new sealed block header, once the block is canonical |
| `logs` | Logs of sealed blocks that match a filter |
| `newPendingTransactions` | Hashes of transactions that enter this node's pending pool and may be propagated; full transactions with `true` |
| `syncing` | Sync status changes |

**Flashblocks types**, available when the node runs with `--flashblocks-url`, whether or not its
feed is connected:

| Type | Parameter | Fires with |
|---|---|---|
| `newFlashblocks` | none | The pre-confirmed block, on every new flashblock |
| `pendingLogs` | a log filter, or none | Logs from the latest flashblock that match the filter |
| `newFlashblockTransactions` | `true`, a log filter, or none | Transactions from the latest flashblock |
| `newFlashblockBatches` | a log filter, or none | The latest flashblock's transactions and logs, in one notification |

The parameter forms:

| Parameter | `pendingLogs` | `newFlashblockTransactions` | `newFlashblockBatches` |
|---|---|---|---|
| none, `null` or `false` | Every log | Hashes | Every transaction |
| `true` | Every log | Full transactions | Every transaction |
| `{}` | Every log | Full transactions that have at least one log | Transactions that have at least one log |
| a log filter | Logs that match | Full transactions with a matching log | Transactions with a matching log, each with all its logs |

A filter's `fromBlock`, `toBlock` and `blockHash` are accepted and ignored. An unknown subscription
type, a malformed parameter, an unknown filter key, more than four topic positions, or a
`blockHash` together with a block range get `-32602`.

## Returns

`string`: the subscription ID, hex-encoded.

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

Each flashblocks type has its own page with its message shape:
[`newFlashblocks`](../flashblocks-api/newFlashblocks.md),
[`pendingLogs`](../flashblocks-api/pendingLogs.md),
[`newFlashblockTransactions`](../flashblocks-api/newFlashblockTransactions.md) and
[`newFlashblockBatches`](../flashblocks-api/newFlashblockBatches.md).

`newHeads` fires on sealed blocks only. It does **not** fire per flashblock. To follow the
pre-confirmed block, use `newFlashblocks`.

## Limits

- One connection holds at most 4 `newFlashblocks` subscriptions, and at most 32 `pendingLogs`,
  `newFlashblockTransactions` and `newFlashblockBatches` subscriptions together. A further one is
  refused with `-32006` and the message `too many newFlashblocks subscriptions on this connection:
  max 4`, or `too many pendingLogs, newFlashblockTransactions and newFlashblockBatches subscriptions
  on this connection: max 32`.
- `newFlashblocks` sends a block only once the one before has left the node's send buffer, and
  drops the updates in between. A dropped update is never sent later. When it was a block's last
  update, the final state of that block never arrives, and the next message is the next block.
- A flashblocks subscription ends without a message 10 s after the messages its client has not
  read fill the connection's network buffers (several MB). On the devnet, one `newFlashblockBatches`
  subscription (about 65 KB/s) ended about 61 s after its client stopped reading. A client that
  reads more slowly than its subscription sends can lose it the same way. `eth_unsubscribe`
  answers `false` for an ended subscription.
- A `pendingLogs`, `newFlashblockTransactions` or `newFlashblockBatches` subscriber that falls more
  than 32 flashblocks behind (6.4 s at the builder's 200 ms) skips the oldest without notice. To
  read what it missed, call `eth_getBlockByNumber("pending", true)` and
  `eth_getBlockReceipts("pending")`, or read the sealed blocks.

## What the flashblocks subscriptions never signal

- **The seal.** No flashblocks subscription sends a message when the node imports the sealed
  block. Follow `newHeads`, then read the sealed block's receipts.
- **A retraction.** `removed` is always `false`. A pre-confirmed transaction that does not land is
  never withdrawn from a stream.
- **A feed fault.** When the node rejects a flashblock, or its feed loses one, the subscriptions
  send nothing more for that block. They go on at the next first flashblock that the builder sends.
- **A rebuild.** When the builder starts the block again, the subscriptions stream it again from
  its first flashblock. See [At a block boundary](../rpc-overview.md#at-a-block-boundary).
