# eth_getBlockReceipts

Returns every transaction receipt in a block.

| | |
|---|---|
| Flashblocks `pending` | **Yes** — `pending` returns the receipts of the flashblock being built |

## Parameters

| Name | Type | Required | Description |
|---|---|---|---|
| `block` | string | yes | Block number in hex, or `"latest"`, `"pending"`, `"safe"`, `"finalized"`, `"earliest"` |

## Returns

Array of receipt objects, each in the shape `eth_getTransactionReceipt` returns.

## Example

**Request**

```json
{
  "jsonrpc": "2.0",
  "method": "eth_getBlockReceipts",
  "params": ["latest"],
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
      "transactionHash": "0x03c8f106f18ad94190e763e21b584c5825b2f4c61f1274c0e8abe65b4476cd51",
      "transactionIndex": "0x0",
      "blockHash": "0x5c330e55a190f82ea486b61e5b12e27dfb4fb3cecfc5746886ef38ca1281bce8",
      "blockNumber": "0x10f2c4",
      "cumulativeGasUsed": "0x5208",
      "gasUsed": "0x5208",
      "status": "0x1",
      "type": "0x2",
      "logs": []
    }
  ]
}
```

## Behaviour at `pending` on this node

`pending` returns the receipts of the flashblock being built, in block order. They are the
receipts that `eth_getTransactionReceipt` returns for each transaction, and they list the
transactions that `eth_getBlockByNumber("pending")` lists, in the same order.

A pre-confirmed receipt carries `blockHash: 0x000…0`, because the block has no hash yet. It is not
proof of inclusion; see [`eth_getTransactionReceipt`](./eth_getTransactionReceipt.md).

Without flashblock data, `pending` returns the receipts of the latest confirmed block.
