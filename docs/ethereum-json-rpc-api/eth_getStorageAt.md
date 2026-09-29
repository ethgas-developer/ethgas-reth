# eth_getStorageAt

Returns the value in a contract storage slot.

| | |
|---|---|
| Flashblocks `pending` | **Yes** |

## Parameters

| Name | Type | Required | Description |
|---|---|---|---|
| `address` | string | yes | The 20-byte address holding the storage |
| `position` | string | yes | The slot position as a hex quantity |
| `block` | string | no | Block number in hex, or a tag. Defaults to `"latest"` |

## Returns

`string` — the 32-byte value at that slot, hex-encoded.

## Example

**Request**

```json
{
  "jsonrpc": "2.0",
  "method": "eth_getStorageAt",
  "params": ["0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48", "0x0", "pending"],
  "id": 1
}
```

**Response**

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": "0x0000000000000000000000000000000000000000000000000000000000000001"
}
```

## Behaviour at `pending` on this node

A slot written by a pre-confirmed transaction reads back at `pending` before its block is sealed,
the same as `eth_getCode`. The value comes from the node's own execution of the flashblock
transactions on top of the latest confirmed block.

A slot written only by a transaction in the node's mempool is never visible at `pending`.

To prove a slot value, use `eth_getProof` at `latest` or at a block number. It refuses `pending`,
because pre-confirmed state has no state root.
