# pendingLogs

Streams logs from pre-confirmed transactions, as they are sequenced.

| | |
|---|---|
| Method | `eth_subscribe` |
| Transport | **WebSocket or IPC** |
| Parameters | an optional log filter |

## Subscribe

With a filter:

```json
{
  "jsonrpc": "2.0",
  "method": "eth_subscribe",
  "params": [
    "pendingLogs",
    {
      "address": "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48",
      "topics": ["0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef"]
    }
  ],
  "id": 1
}
```

Without one, or with `true`, `false`, `null` or `{}`, every pre-confirmed log is streamed. A
filter's block fields are ignored; see
[`eth_subscribe`](../ethereum-json-rpc-api/eth_subscribe.md#parameters).

| Filter field | Type | Description |
|---|---|---|
| `address` | string \| array | Contract address or addresses to match |
| `topics` | array | Topic filters, in the same form
[`eth_getLogs`](../ethereum-json-rpc-api/eth_getLogs.md) takes |

## Notifications

**One log per message.** Only logs from the most recent flashblock are emitted, so a log is not
repeated as the block grows, unless the builder starts the block again.

```json
{
  "jsonrpc": "2.0",
  "method": "eth_subscription",
  "params": {
    "subscription": "0x2a7bc8d4e3f5a6b1c2d3e4f5a6b7c8d9",
    "result": {
      "address": "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48",
      "topics": ["0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef"],
      "data": "0x0000000000000000000000000000000000000000000000000de0b6b3a7640000",
      "blockNumber": "0x10f2c5",
      "blockTimestamp": "0x68cf1a3c",
      "blockHash": "0x0000000000000000000000000000000000000000000000000000000000000000",
      "transactionHash": "0x7f4e2a8c1b6d9035e4a7c2f8b1d6e9a34c7f0b5d8e2a6c9f3b7d1e4a8c2f6b0c",
      "transactionIndex": "0x0",
      "logIndex": "0x0",
      "removed": false
    }
  }
}
```

## Notes

**`blockTimestamp` is the timestamp of the block being built.**

**`blockHash` is `0x000…0` on every pre-confirmed log.** Do not group by it: every pre-confirmed log
would fall into one group. Group by `transactionHash` instead.

**A pre-confirmed log can still disappear, and no message withdraws it**: `removed` is always
`false`. Reconcile against the standard `logs` subscription, which fires on sealed blocks.

**Nothing marks where a flashblock ends.** To receive each flashblock's logs in one notification,
subscribe to [`newFlashblockBatches`](./newFlashblockBatches.md) instead.

**Limits**, and what the flashblocks subscriptions never signal:
[`eth_subscribe`](../ethereum-json-rpc-api/eth_subscribe.md#limits).
