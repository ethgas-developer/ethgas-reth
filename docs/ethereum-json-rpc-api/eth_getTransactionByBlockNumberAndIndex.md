# eth_getTransactionByBlockNumberAndIndex

Returns the transaction at a given index within the block with the given number or tag.

| | |
|---|---|
| At `pending` | **Pre-confirmed** |

## Parameters

| Name | Type | Required | Description |
|---|---|---|---|
| `block` | string | yes | Block number in hex, or `"latest"`, `"pending"`, `"safe"`, `"finalized"`, `"earliest"` |
| `index` | string | yes | The index within the block, as a hex quantity |

## Returns

`object | null`: a transaction object, or `null` if not found.

## Example

**Request**

```json
{
  "jsonrpc": "2.0",
  "method": "eth_getTransactionByBlockNumberAndIndex",
  "params": ["latest", "0x0"],
  "id": 1
}
```

**Response**

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "hash": "0x03c8f106f18ad94190e763e21b584c5825b2f4c61f1274c0e8abe65b4476cd51",
    "blockHash": "0x5c330e55a190f82ea486b61e5b12e27dfb4fb3cecfc5746886ef38ca1281bce8",
    "blockNumber": "0x10f2c4",
    "transactionIndex": "0x0",
    "from": "0xd3cda913deb6f4967b2ef66ae97de114a83bcc01",
    "to": "0x742d35cc6634c0532925a3b8d4c9dd0b4f3baea1",
    "value": "0x2c68af0bb14000",
    "gas": "0x5208",
    "maxFeePerGas": "0x4c4b40",
    "maxPriorityFeePerGas": "0xf4240",
    "nonce": "0x4d2",
    "input": "0x",
    "type": "0x2",
    "chainId": "0x88bb0"
  }
}
```

## Behaviour at `pending` on this node

`pending` returns the transaction at that index in the pre-confirmed block, counted from the
block's first transaction: the transaction
that `eth_getBlockByNumber("pending", true)` lists at that index. Every index below the count that
`eth_getBlockTransactionCountByNumber("pending")` reports is retrievable.

A pre-confirmed transaction has `blockHash: null` and a real `blockNumber` and `transactionIndex`.

Without flashblock data, `pending` indexes the next block when the node has already executed it
for its consensus client, else the latest block.
