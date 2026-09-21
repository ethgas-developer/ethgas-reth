# eth_getTransactionByBlockNumberAndIndex

Returns the transaction at a given index within the block with the given number or tag.

| | |
|---|---|
| Flashblocks `pending` | **No** — `pending` returns the latest confirmed block |

## Parameters

| Name | Type | Required | Description |
|---|---|---|---|
| `block` | string | yes | Block number in hex, or `"latest"`, `"pending"`, `"safe"`, `"finalized"`, `"earliest"` |
| `index` | string | yes | The index within the block, as a hex quantity |

## Returns

`object | null` — a transaction object, or `null` if not found.

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
    "from": "0xd3CdA913deB6f4967b2Ef66ae97DE114a83bcc01",
    "to": "0x742d35Cc6634C0532925a3b8D4C9dD0b4f3BaEa",
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

`pending` returns the latest confirmed block, so this method does not return pre-confirmed
transactions.

This disagrees with `eth_getBlockTransactionCountByNumber`, which reports the flashblock count `N`
at `pending`. Indices `0..N-1` are therefore not all retrievable here. The
mismatch is known. Use `eth_getBlockByNumber("pending", true)` to read the pending transaction
list.
