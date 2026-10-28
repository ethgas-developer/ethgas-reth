# newFlashblockBatches

Streams each flashblock as one notification: the transactions it added, with their logs.

| | |
|---|---|
| Method | `eth_subscribe` |
| Transport | **WebSocket or IPC** |
| Parameters | none, or a log filter |

Use it when you need to know that you have all of a flashblock.
[`newFlashblockTransactions`](./newFlashblockTransactions.md) and [`pendingLogs`](./pendingLogs.md)
send one notification per transaction or per log, and nothing marks where a flashblock ends.

## Subscribe

| Parameter | `transactions` holds |
|---|---|
| none, `null`, `true` or `false` | Every transaction the flashblock added |
| a log filter | The transactions where at least one log matches |
| `{}` | The transactions that have at least one log |

A filter's block fields are ignored; see
[`eth_subscribe`](../ethereum-json-rpc-api/eth_subscribe.md#parameters).

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
      "address": "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48",
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
          "hash": "0x7f4e2a8c1b6d9035e4a7c2f8b1d6e9a34c7f0b5d8e2a6c9f3b7d1e4a8c2f6b0c",
          "blockHash": null,
          "blockNumber": "0x10f2c5",
          "blockTimestamp": "0x68cf1a3c",
          "transactionIndex": "0x7",
          "from": "0xd3cda913deb6f4967b2ef66ae97de114a83bcc01",
          "to": "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48",
          "input": "0xa9059cbb...",
          "type": "0x2",
          "logs": [
            {
              "address": "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48",
              "topics": ["0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef"],
              "data": "0x0000000000000000000000000000000000000000000000000de0b6b3a7640000",
              "blockHash": "0x0000000000000000000000000000000000000000000000000000000000000000",
              "blockNumber": "0x10f2c5",
              "blockTimestamp": "0x68cf1a3c",
              "transactionHash": "0x7f4e2a8c1b6d9035e4a7c2f8b1d6e9a34c7f0b5d8e2a6c9f3b7d1e4a8c2f6b0c",
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

**`flashblockIndex` back at `0x0` on the same `blockNumber` means the builder started the block
again.** Discard what you collected from the earlier build: its transactions can land in another
order, or not at all.

**`transactionIndex` and `logIndex` count from the start of the block**, not from the start of the
flashblock.

**`blockHash` is `null` on each transaction and `0x000…0` on each log**, as on the other flashblock
subscriptions. Group by `blockNumber` and `flashblockIndex` instead.

**A new subscription starts at the next flashblock.** To catch up after a reconnect, read the
pre-confirmed block once:
[`eth_getBlockByNumber`](../ethereum-json-rpc-api/eth_getBlockByNumber.md) with `"pending"` and
`true`, and [`eth_getBlockReceipts`](../ethereum-json-rpc-api/eth_getBlockReceipts.md) with
`"pending"`. Together they hold every transaction of the pre-confirmed block so far, with its
receipt fields and logs, in block order; apply your filter to the logs yourself.
[`eth_getLogs`](../ethereum-json-rpc-api/eth_getLogs.md) at `"pending"` returns only logs, so it misses
every transaction without one. The read can include a flashblock you then also receive here.
Deduplicate by the transaction `hash`.

**Sequenced is not sealed.** A transaction here has been selected by the builder. It is not yet in
a sealed block. To reconcile, follow `newHeads` and read each sealed block's receipts with
`eth_getBlockReceipts`; the standard `logs` subscription misses every transaction without a log.

**Limits**, and what the flashblocks subscriptions never signal:
[`eth_subscribe`](../ethereum-json-rpc-api/eth_subscribe.md#limits).
