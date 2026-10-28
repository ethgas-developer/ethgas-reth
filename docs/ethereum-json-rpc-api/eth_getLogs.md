# eth_getLogs

Returns the logs matching a filter.

| | |
|---|---|
| At `pending` | **Pre-confirmed** when `toBlock` is `pending` |

## Parameters

| Name | Type | Required | Description |
|---|---|---|---|
| `filter` | object | yes | Filter options. Every field is optional |

Filter fields:

| Field | Type | Description |
|---|---|---|
| `fromBlock` | string | Range start. Hex or tag. Defaults to `"latest"` |
| `toBlock` | string | Range end. Hex or tag. Defaults to `"latest"`. Write `"pending"` out to reach flashblock logs |
| `address` | string \| array | Contract address or addresses to match |
| `topics` | array | Topic filters, at most four positions. Accepts `null`, a hex string, or an array of hex strings per position |
| `blockHash` | string | Restricts to one block. With `fromBlock` or `toBlock` as well, the request is refused with `-32602` |

## Returns

Array of log objects, each with `address`, `topics`, `data`, `blockHash`, `blockNumber`,
`blockTimestamp`, `transactionHash`, `transactionIndex`, `logIndex` and `removed`.

## Example

**Request**

```json
{
  "jsonrpc": "2.0",
  "method": "eth_getLogs",
  "params": [{
    "fromBlock": "pending",
    "toBlock": "pending",
    "address": "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48",
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
      "address": "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48",
      "topics": [
        "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef",
        "0x000000000000000000000000d3cda913deb6f4967b2ef66ae97de114a83bcc01",
        "0x000000000000000000000000742d35cc6634c0532925a3b8d4c9dd0b4f3baea1"
      ],
      "data": "0x0000000000000000000000000000000000000000000000000de0b6b3a7640000",
      "blockHash": "0x0000000000000000000000000000000000000000000000000000000000000000",
      "blockNumber": "0x10f2c5",
      "blockTimestamp": "0x68cf1a3c",
      "transactionHash": "0x7f4e2a8c1b6d9035e4a7c2f8b1d6e9a34c7f0b5d8e2a6c9f3b7d1e4a8c2f6b0c",
      "transactionIndex": "0x0",
      "logIndex": "0x0",
      "removed": false
    }
  ]
}
```

## Behaviour at `pending` on this node

A filter whose `toBlock` is `pending` includes logs from pre-confirmed transactions.

**The trigger is `toBlock`, written out.** These reach flashblocks:

```json
{"fromBlock": "pending", "toBlock": "pending"}
```

```json
{"fromBlock": "0x10f2c0", "toBlock": "pending"}
```

These do not:

```json
{"fromBlock": "pending"}
```

```json
{"fromBlock": "pending", "toBlock": "latest"}
```

`{"fromBlock": "pending"}` alone answers from the block the node is importing, else from the latest
block. `{"fromBlock": "pending", "toBlock": "latest"}` returns `[]`. A `blockHash` filter names a
sealed block.

A numeric `fromBlock` must be at most the latest block number. A range from the pre-confirmed
block's own number to `pending` is refused with `-32602` and `invalid block range params`.

**`pending` to `pending` returns the logs of every pre-confirmed block.** In the
[two-block window](../rpc-overview.md#at-a-block-boundary) these are the logs of two blocks; tell
them apart by `blockNumber`.

**Pre-confirmed logs carry `blockHash: 0x000…0`,** because the block has no hash yet. A consumer
that groups logs by `blockHash` puts every pre-confirmed log into one group. Group by
`transactionHash` instead while a log is pre-confirmed.

**Without flashblock data,** `{"fromBlock": "pending", "toBlock": "pending"}` returns the logs of
the block the node is importing, else of the latest block. A range that starts below `pending`
ends at the latest block.

**A filter never includes flashblock logs.** `eth_newFilter`, read with `eth_getFilterLogs` or
`eth_getFilterChanges`, returns no flashblock logs. Poll with `eth_getLogs` instead.

To stream pre-confirmed logs rather than poll, subscribe to `pendingLogs`. To receive each
flashblock's logs in one notification, subscribe to `newFlashblockBatches`.
