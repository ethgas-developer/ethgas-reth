# newFlashblockBatches

Streams each flashblock as one notification: the transactions it added, with their logs.

| | |
|---|---|
| Method | `eth_subscribe` |
| Transport | **WebSocket only** |
| Parameters | none, or a log filter |

Use it when you need to know that you have all of a flashblock.
[`newFlashblockTransactions`](./newFlashblockTransactions.md) and [`pendingLogs`](./pendingLogs.md)
send one notification per transaction or per log, and nothing marks where a flashblock ends.

## Subscribe

| Parameter | `transactions` holds |
|---|---|
| none | Every transaction the flashblock added |
| a log filter | The transactions where at least one log matches |

**Every transaction**

```json
{
  "jsonrpc": "2.0",
  "method": "eth_subscribe",
  "params": ["newFlashblockBatches"],
  "id": 1
}
```

**Filtered by log**

```json
{
  "jsonrpc": "2.0",
  "method": "eth_subscribe",
  "params": [
    "newFlashblockBatches",
    {
      "address": "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48",
      "topics": ["0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef"]
    }
  ],
  "id": 1
}
```

> **The filter selects transactions, it does not trim logs.** A transaction is included when **any**
> of its logs matches, and it then carries **all** of its logs, not only the matching ones.

## Notifications

One notification per flashblock, **even when no transaction matches**.

| Field | Type | Notes |
|---|---|---|
| `blockNumber` | hex quantity | The block the flashblock belongs to |
| `flashblockIndex` | hex quantity | The flashblock's position in its block, from `0x0` |
| `transactions` | array | The transactions the flashblock added, in block order. Can be empty |

Each entry in `transactions` is the object that
[`newFlashblockTransactions`](./newFlashblockTransactions.md) sends with `true`: the full
transaction, flattened, plus `logs`, `logsBloom`, `gasUsed`, `cumulativeGasUsed`, `status` and
`contractAddress`. The transaction fields are shortened in this example.

```json
{
  "jsonrpc": "2.0",
  "method": "eth_subscription",
  "params": {
    "subscription": "0x1887ec8b9589ccad00000000000532da",
    "result": {
      "blockNumber": "0x10f2c5",
      "flashblockIndex": "0x3",
      "transactions": [
        {
          "hash": "0x7f4e2a8c1b6d9035e4a7c2f8b1d6e9a34c7f0b5d8e2a6c9f3b7d1e4a8c2f6b0c3",
          "blockHash": null,
          "blockNumber": "0x10f2c5",
          "blockTimestamp": "0x68cf1a3c",
          "transactionIndex": "0x7",
          "from": "0xd3CdA913deB6f4967b2Ef66ae97DE114a83bcc01",
          "to": "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48",
          "input": "0xa9059cbb...",
          "type": "0x2",
          "logs": [
            {
              "address": "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48",
              "topics": ["0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef"],
              "data": "0x0000000000000000000000000000000000000000000000000de0b6b3a7640000",
              "blockHash": "0x0000000000000000000000000000000000000000000000000000000000000000",
              "blockNumber": "0x10f2c5",
              "blockTimestamp": "0x68cf1a3c",
              "transactionHash": "0x7f4e2a8c1b6d9035e4a7c2f8b1d6e9a34c7f0b5d8e2a6c9f3b7d1e4a8c2f6b0c3",
              "transactionIndex": "0x7",
              "logIndex": "0xc",
              "removed": false
            }
          ],
          "logsBloom": "0x0000...0000",
          "gasUsed": "0xb48a",
          "cumulativeGasUsed": "0x7a3f2",
          "status": "0x1",
          "contractAddress": null
        }
      ]
    }
  }
}
```

## Notes

**Each notification is a complete flashblock.** The subscription sends one notification for every
flashblock the node applies, even an empty one, unless your client falls behind. You do not need to
wait for the next flashblock to know this one is done.

**A gap in `flashblockIndex` within a block means you missed a flashblock**, for example because
your client fell behind. See the limits below.

**`flashblockIndex` back at `0x0` on the same `blockNumber` means the builder rebuilt the block.**
This happens after a missed slot. Discard what you collected from the earlier build. Its
transactions will not land as they were sequenced.

**`transactionIndex` and `logIndex` count from the start of the block**, not from the start of the
flashblock.

**`blockHash` is `null` on each transaction and `0x000…0` on each log**, as on the other flashblock
subscriptions. Group by `blockNumber` and `flashblockIndex` instead.

**A new subscription starts at the next flashblock.** To catch up after a reconnect, call
[`eth_getLogs`](../ethereum-json-rpc-api/eth_getLogs.md) once with `fromBlock` and `toBlock` set to
`"pending"`. It returns the logs of every flashblock in the pending block so far, and can include a
flashblock you then also receive here. Deduplicate by `transactionHash` and `logIndex`.

**Sequenced is not sealed.** A transaction here has been selected by the builder, not included in a
confirmed block. Reconcile against the standard `logs` subscription, which fires on confirmed blocks.

**Limits.** One connection holds at most 4 `newFlashblocks` subscriptions and 32 of the other three
kinds, a subscription whose client stops reading ends, and a subscriber that falls behind skips
flashblocks. See
[`eth_subscribe`](../ethereum-json-rpc-api/eth_subscribe.md#limits).
