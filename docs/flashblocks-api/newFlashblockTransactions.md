# newFlashblockTransactions

Streams each transaction as it is sequenced into a flashblock.

| | |
|---|---|
| Method | `eth_subscribe` |
| Transport | **WebSocket or IPC** |
| Parameters | none, `true`, or a log filter |

A transaction here has been sequenced by the builder. The standard `newPendingTransactions`
subscription streams the transactions that enter this node's pending pool instead.

## Subscribe

The parameter decides what you receive.

| Parameter | You receive |
|---|---|
| none, `null` or `false` | Transaction hashes only |
| `true` | The full transaction, plus the receipt fields already known |
| a log filter | The full transaction, for transactions where at least one log matches |
| `{}` | The full transaction, for transactions that have at least one log |

A filter's block fields are ignored; see
[`eth_subscribe`](../ethereum-json-rpc-api/eth_subscribe.md#parameters).

**Hashes only**

```json
{
  "jsonrpc": "2.0",
  "method": "eth_subscribe",
  "params": ["newFlashblockTransactions"],
  "id": 1
}
```

**Full transactions**

```json
{
  "jsonrpc": "2.0",
  "method": "eth_subscribe",
  "params": ["newFlashblockTransactions", true],
  "id": 1
}
```

**Full transactions, filtered by log**

```json
{
  "jsonrpc": "2.0",
  "method": "eth_subscribe",
  "params": [
    "newFlashblockTransactions",
    {
      "address": "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48",
      "topics": ["0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef"]
    }
  ],
  "id": 1
}
```

> **The filter selects transactions, it does not trim logs.** A transaction is emitted when **any**
> of its logs matches, and the notification then carries **all** of that transaction's logs, not
> only the matching ones.

## Notifications

**Hashes only:**

```json
{
  "jsonrpc": "2.0",
  "method": "eth_subscription",
  "params": {
    "subscription": "0x1887ec8b9589ccad00000000000532da",
    "result": "0x7f4e2a8c1b6d9035e4a7c2f8b1d6e9a34c7f0b5d8e2a6c9f3b7d1e4a8c2f6b0c"
  }
}
```

**Full form.** The transaction object is flattened in, and these receipt fields are added, each
serialized exactly as
[`eth_getTransactionReceipt`](../ethereum-json-rpc-api/eth_getTransactionReceipt.md) serializes it:

| Field | Type | Notes |
|---|---|---|
| `logs` | array | Every log this transaction emitted |
| `logsBloom` | 256-byte bloom | Bloom over `logs` |
| `gasUsed` | hex quantity | Gas this transaction alone consumed |
| `cumulativeGasUsed` | hex quantity | Gas used by the block up to and including this transaction |
| `status` | hex quantity | `0x1` or `0x0` |
| `contractAddress` | address \| null | Set when the transaction creates a contract (`to` is `null`), also when the creation failed |

```json
{
  "jsonrpc": "2.0",
  "method": "eth_subscription",
  "params": {
    "subscription": "0x1887ec8b9589ccad00000000000532da",
    "result": {
      "hash": "0x7f4e2a8c1b6d9035e4a7c2f8b1d6e9a34c7f0b5d8e2a6c9f3b7d1e4a8c2f6b0c",
      "blockHash": null,
      "blockNumber": "0x10f2c5",
      "transactionIndex": "0x0",
      "blockTimestamp": "0x68cf1a3c",
      "from": "0xd3cda913deb6f4967b2ef66ae97de114a83bcc01",
      "to": "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48",
      "value": "0x0",
      "gas": "0x186a0",
      "maxFeePerGas": "0x4c4b40",
      "maxPriorityFeePerGas": "0xf4240",
      "nonce": "0x4d2",
      "input": "0xa9059cbb...",
      "type": "0x2",
      "chainId": "0x88bb0",
      "logs": [
        {
          "address": "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48",
          "topics": ["0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef"],
          "data": "0x0000000000000000000000000000000000000000000000000de0b6b3a7640000",
          "blockHash": "0x0000000000000000000000000000000000000000000000000000000000000000",
          "blockNumber": "0x10f2c5",
          "blockTimestamp": "0x68cf1a3c",
          "transactionHash": "0x7f4e2a8c1b6d9035e4a7c2f8b1d6e9a34c7f0b5d8e2a6c9f3b7d1e4a8c2f6b0c",
          "transactionIndex": "0x0",
          "logIndex": "0x0",
          "removed": false
        }
      ],
      "logsBloom": "0x0000...0000",
      "gasUsed": "0xb48a",
      "cumulativeGasUsed": "0xb48a",
      "status": "0x1",
      "contractAddress": null
    }
  }
}
```

## Notes

**`blockHash` is `null` on the transaction and `0x000…0` on its logs.** The block is not sealed.

**Sequenced is not sealed.** A transaction here has been selected by the builder. It is not yet in
a sealed block.

**`transactionIndex` and `logIndex` count from the start of the block**, not from the start of the
flashblock.

**Nothing marks where a flashblock ends.** To receive each flashblock's transactions in one
notification, subscribe to [`newFlashblockBatches`](./newFlashblockBatches.md) instead.

**Limits**, and what the flashblocks subscriptions never signal:
[`eth_subscribe`](../ethereum-json-rpc-api/eth_subscribe.md#limits).
