# newFlashblocks

Streams the pending block each time a new flashblock is applied to it.

| | |
|---|---|
| Method | `eth_subscribe` |
| Transport | **WebSocket or IPC** |
| Parameters | none |

## Subscribe

```json
{
  "jsonrpc": "2.0",
  "method": "eth_subscribe",
  "params": ["newFlashblocks"],
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

## Notifications

Each notification carries **the whole pending block, with full transaction objects** — not the raw
flashblock that triggered it. Every flashblock produces one notification, so you will receive
several notifications for the same block height as it grows.

```json
{
  "jsonrpc": "2.0",
  "method": "eth_subscription",
  "params": {
    "subscription": "0x1887ec8b9589ccad00000000000532da",
    "result": {
      "number": "0x10f2c5",
      "hash": "0x0000000000000000000000000000000000000000000000000000000000000000",
      "parentHash": "0x5c330e55a190f82ea486b61e5b12e27dfb4fb3cecfc5746886ef38ca1281bce8",
      "timestamp": "0x68cf1a3c",
      "gasLimit": "0x1c9c380",
      "gasUsed": "0x5208",
      "baseFeePerGas": "0x3b9aca00",
      "transactions": [
        {
          "hash": "0x7f4e2a8c1b6d9035e4a7c2f8b1d6e9a34c7f0b5d8e2a6c9f3b7d1e4a8c2f6b0c3",
          "blockHash": null,
          "blockNumber": "0x10f2c5",
          "transactionIndex": "0x0",
          "blockTimestamp": "0x68cf1a3c",
          "from": "0xd3CdA913deB6f4967b2Ef66ae97DE114a83bcc01",
          "to": "0x742d35Cc6634C0532925a3b8D4C9dD0b4f3BaEa",
          "value": "0x2c68af0bb14000",
          "type": "0x2"
        }
      ],
      "uncles": []
    }
  }
}
```

## Notes

**The block is cumulative, not a delta.** Each notification contains every transaction sequenced
into that block so far. If you want only what is new, subscribe to
[`newFlashblockTransactions`](./newFlashblockTransactions.md) instead.

**`hash` is `0x000…0` and each transaction's `blockHash` is `null`.** The block is not sealed.

**After the Amsterdam fork, the block carries `slotNumber` and no `blockAccessListHash`**, as
[`eth_getBlockByNumber`](../ethereum-json-rpc-api/eth_getBlockByNumber.md) describes.

**To follow confirmed blocks instead**, use the standard `newHeads` subscription. It does not fire
per flashblock.

**Limits.** One connection holds at most 4 `newFlashblocks` subscriptions and 32 of the other two
kinds, a subscription whose client stops reading ends, and a subscriber that falls behind skips
flashblocks. See
[`eth_subscribe`](../ethereum-json-rpc-api/eth_subscribe.md#limits).
