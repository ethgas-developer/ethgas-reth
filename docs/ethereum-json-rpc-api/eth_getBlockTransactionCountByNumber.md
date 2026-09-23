# eth_getBlockTransactionCountByNumber

Returns the number of transactions in the block with the given number or tag.

| | |
|---|---|
| Flashblocks `pending` | **Yes** — `pending` reflects the flashblock being built |

## Parameters

| Name | Type | Required | Description |
|---|---|---|---|
| `block` | string | yes | Block number in hex, or `"latest"`, `"pending"`, `"safe"`, `"finalized"`, `"earliest"` |

## Returns

`string | null` — the transaction count as a hex quantity.

## Example

**Request**

```json
{
  "jsonrpc": "2.0",
  "method": "eth_getBlockTransactionCountByNumber",
  "params": ["pending"],
  "id": 1
}
```

**Response**

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": "0x1f"
}
```

## Behaviour at `pending` on this node

`pending` returns the transaction count of the flashblock being built.

**Mind the gap against `eth_getTransactionByBlockNumberAndIndex`.** That method answers `pending`
from the latest confirmed block. The count you get here and the range of indices you can actually
retrieve there do not agree at `pending`.
