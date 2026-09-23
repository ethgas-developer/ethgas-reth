# pendingLogs

Streams logs from pre-confirmed transactions, as they are sequenced.

| | |
|---|---|
| Method | `eth_subscribe` |
| Transport | **WebSocket only** |
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
      "address": "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48",
      "topics": ["0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef"]
    }
  ],
  "id": 1
}
```

Without one, every pre-confirmed log is streamed.

| Filter field | Type | Description |
|---|---|---|
| `address` | string \| array | Contract address or addresses to match |
| `topics` | array | Topic filters, in the same form
[`eth_getLogs`](../ethereum-json-rpc-api/eth_getLogs.md) takes |

## Notifications

**One log per message.** Only logs from the most recent flashblock are emitted, so a log is not
repeated as the block grows.

```json
{
  "jsonrpc": "2.0",
  "method": "eth_subscription",
  "params": {
    "subscription": "0x2a7bc8d4e3f5a6b1c2d3e4f5a6b7c8d9",
    "result": {
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
  }
}
```

## Notes

**`blockHash` is `0x000…0` on every pre-confirmed log.** Do not group by it — every pending log
would collapse into one bucket. Group by `transactionHash` instead.

**A pre-confirmed log can still disappear.** Nothing is final until the block is sealed. Treat these
as early signals and reconcile against the standard `logs` subscription, which fires on confirmed
blocks.
