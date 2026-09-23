# eth_getLogs

Returns the logs matching a filter.

| | |
|---|---|
| Flashblocks `pending` | **Yes** — `pending` reflects the flashblock being built |

## Parameters

| Name | Type | Required | Description |
|---|---|---|---|
| `filter` | object | yes | Filter options, at least one criterion |

Filter fields:

| Field | Type | Description |
|---|---|---|
| `fromBlock` | string | Range start. Hex or tag. Defaults to `"latest"` |
| `toBlock` | string | Range end. Hex or tag. Defaults to `"latest"` |
| `address` | string \| array | Contract address or addresses to match |
| `topics` | array | Topic filters. Accepts `null`, a hex string, or an array of hex strings per position |
| `blockHash` | string | Restricts to one block. Overrides `fromBlock` and `toBlock` |

## Returns

Array of log objects, each with `address`, `topics`, `data`, `blockNumber`, `blockHash`,
`transactionHash`, `transactionIndex`, `logIndex` and `removed`.

## Example

**Request**

```json
{
  "jsonrpc": "2.0",
  "method": "eth_getLogs",
  "params": [{
    "fromBlock": "pending",
    "toBlock": "pending",
    "address": "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48",
    "topics": ["0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef"]
  }],
  "id": 1
}
```

**Response**

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": [
    {
      "address": "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48",
      "topics": ["0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef"],
      "data": "0x0000000000000000000000000000000000000000000000000de0b6b3a7640000",
      "blockNumber": "0x10f2c5",
      "blockHash": "0x0000000000000000000000000000000000000000000000000000000000000000",
      "transactionHash": "0x7f4e2a8c1b6d9035e4a7c2f8b1d6e9a34c7f0b5d8e2a6c9f3b7d1e4a8c2f6b0c3",
      "transactionIndex": "0x0",
      "logIndex": "0x0",
      "removed": false
    }
  ]
}
```

## Behaviour at `pending` on this node

A filter that reaches `pending` includes logs from pre-confirmed transactions.

**The trigger is `toBlock`, not `fromBlock`.** The override engages only when `toBlock` is
`pending`. A filter with `"fromBlock": "pending"` but `"toBlock": "latest"` returns no flashblock
logs. A `blockHash` filter behaves the same way, since it names a sealed block.

So this reaches flashblocks:

```json
{"fromBlock": "pending", "toBlock": "pending"}
```

and so does a range that ends at pending:

```json
{"fromBlock": "0x10f2c0", "toBlock": "pending"}
```

but this does not:

```json
{"fromBlock": "pending", "toBlock": "latest"}
```

**Pending logs carry `blockHash: 0x000…0`,** because the flashblock has no hash. A consumer that
groups logs by `blockHash` will collapse every pending log into one bucket. Group by
`transactionHash` instead while a log is pending.

To stream pending logs rather than poll, subscribe to `pendingLogs`.
