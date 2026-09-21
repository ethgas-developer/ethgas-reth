# eth_getBlockByNumber

Returns a block by number or tag.

| | |
|---|---|
| Flashblocks `pending` | **Yes** — `pending` reflects the flashblock being built |

## Parameters

| Name | Type | Required | Description |
|---|---|---|---|
| `block` | string | yes | Block number in hex, or `"latest"`, `"pending"`, `"safe"`, `"finalized"`, `"earliest"` |
| `fullTransactions` | boolean | yes | `true` for full transaction objects, `false` for hashes only |

## Returns

`object | null` — a block object, or `null` if no such block exists.

## Example

**Request**

```json
{
  "jsonrpc": "2.0",
  "method": "eth_getBlockByNumber",
  "params": ["pending", false],
  "id": 1
}
```

**Response**

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "number": "0x10f2c5",
    "hash": "0x0000000000000000000000000000000000000000000000000000000000000000",
    "parentHash": "0x5c330e55a190f82ea486b61e5b12e27dfb4fb3cecfc5746886ef38ca1281bce8",
    "timestamp": "0x68cf1a3c",
    "gasLimit": "0x1c9c380",
    "gasUsed": "0x5208",
    "baseFeePerGas": "0x3b9aca00",
    "transactions": ["0x7f4e2a8c1b6d9035e4a7c2f8b1d6e9a34c7f0b5d8e2a6c9f3b7d1e4a8c2f6b0c3"],
    "uncles": []
  }
}
```

## Behaviour at `pending` on this node

`pending` returns the block assembled from the flashblocks received so far.

**Its `hash` is `0x000…0`.** The node does not compute a hash for an unsealed block, and does not
publish the producer's. Do not key on it, and do not pass it to `eth_getBlockByHash`.

If no flashblock data is available, `pending` returns the latest confirmed block rather than `null`.
