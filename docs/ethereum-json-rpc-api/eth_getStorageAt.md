# eth_getStorageAt

Returns the value in a contract storage slot.

| | |
|---|---|
| Flashblocks `pending` | **Not yet** — planned; today `pending` returns the latest confirmed block |

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

> **Not available yet.** Today `pending` returns the latest confirmed block for this method, so it
> will not show changes made by transactions that are only pre-confirmed. The behaviour described
> below is planned.

Once available, a slot written by a pre-confirmed transaction will read back at `pending` before its
block is sealed, the same as `eth_getCode`.
