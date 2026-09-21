# eth_getBlockByHash

Returns a block by its hash.

| | |
|---|---|
| Flashblocks `pending` | n/a — this method takes no block parameter |

## Parameters

| Name | Type | Required | Description |
|---|---|---|---|
| `blockHash` | string | yes | The 32-byte block hash |
| `fullTransactions` | boolean | yes | `true` for full transaction objects, `false` for hashes only |

## Returns

`object | null` — a block object, or `null` if no such block exists. Same shape as `eth_getBlockByNumber`.

## Example

**Request**

```json
{
  "jsonrpc": "2.0",
  "method": "eth_getBlockByHash",
  "params": ["0x5c330e55a190f82ea486b61e5b12e27dfb4fb3cecfc5746886ef38ca1281bce8", false],
  "id": 1
}
```

**Response**

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "number": "0x10f2c4",
    "hash": "0x5c330e55a190f82ea486b61e5b12e27dfb4fb3cecfc5746886ef38ca1281bce8",
    "parentHash": "0x8f2e6b1c9d4a7530e8b2f1c6a9d3e7b40c5f8a2d1e6b9c4f7a0d3e6b9c2f5a81",
    "timestamp": "0x68cf1a30",
    "gasLimit": "0x1c9c380",
    "gasUsed": "0x2155bc7",
    "baseFeePerGas": "0x4c4b40",
    "transactions": ["0x03c8f106f18ad94190e763e21b584c5825b2f4c61f1274c0e8abe65b4476cd51"],
    "uncles": []
  }
}
```

## Behaviour at `pending` on this node

This method takes a block **hash**, so the `pending` tag does not apply.

It also cannot be used to fetch the pending block, because the pending block has no hash on this
node — it is sealed with `0x000…0`.
