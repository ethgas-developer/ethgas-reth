# newFlashblockTransactions

Streams each transaction as it is sequenced into a flashblock.

| | |
|---|---|
| Method | `eth_subscribe` |
| Transport | **WebSocket only** |
| Parameters | none, `true`, or a log filter |

This carries far more inclusion confidence than the standard `newPendingTransactions`
subscription, which streams whatever reaches the node's mempool. A transaction here has been
selected by the builder.

## Subscribe

The parameter decides what you receive. There are three forms.

| Parameter | You receive |
|---|---|
| none | Transaction hashes only |
| `true` | The full transaction, plus the receipt fields already known |
| a log filter | The full transaction, for transactions where at least one log matches |

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
      "address": "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48",
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
    "result": "0x7f4e2a8c1b6d9035e4a7c2f8b1d6e9a34c7f0b5d8e2a6c9f3b7d1e4a8c2f6b0c3"
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
| `contractAddress` | address \| null | Set only when this transaction deployed a contract |

```json
{
  "jsonrpc": "2.0",
  "method": "eth_subscription",
  "params": {
    "subscription": "0x1887ec8b9589ccad00000000000532da",
    "result": {
      "hash": "0x7f4e2a8c1b6d9035e4a7c2f8b1d6e9a34c7f0b5d8e2a6c9f3b7d1e4a8c2f6b0c3",
      "blockHash": null,
      "blockNumber": "0x10f2c5",
      "transactionIndex": "0x0",
      "from": "0xd3CdA913deB6f4967b2Ef66ae97DE114a83bcc01",
      "to": "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48",
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
          "address": "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48",
          "topics": ["0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef"],
          "data": "0x0000000000000000000000000000000000000000000000000de0b6b3a7640000",
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

**`blockHash` is `null`** while a transaction is only pre-confirmed, even though `blockNumber` is
set. That pairing is how you tell a pre-confirmed transaction from a confirmed one.

**Sequenced is not sealed.** A transaction here has been selected by the builder, not included in a
confirmed block.
