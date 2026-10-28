# eth_getBlockTransactionCountByNumber

Returns the number of transactions in the block with the given number or tag.

| | |
|---|---|
| At `pending` | **Pre-confirmed** |

## Parameters

| Name | Type | Required | Description |
|---|---|---|---|
| `block` | string | yes | Block number in hex, or `"latest"`, `"pending"`, `"safe"`, `"finalized"`, `"earliest"` |

## Returns

`string | null`: the transaction count as a hex quantity.

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

`pending` returns the transaction count of the pre-confirmed block, across all its flashblocks.
Every index below it is retrievable with
`eth_getTransactionByBlockNumberAndIndex("pending", index)`.

If no flashblock data is available, `pending` counts the next block when the node has already
executed it for its consensus client, else the latest block.
