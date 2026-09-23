# eth_getCode

Returns the contract bytecode deployed at an address.

| | |
|---|---|
| Flashblocks `pending` | **Not yet** — planned; today `pending` returns the latest confirmed block |

## Parameters

| Name | Type | Required | Description |
|---|---|---|---|
| `address` | string | yes | The 20-byte address to query |
| `block` | string | no | Block number in hex, or a tag. Defaults to `"latest"` |

## Returns

`string` — the bytecode as a hex string, or `"0x"` when the account holds no code.

## Example

**Request**

```json
{
  "jsonrpc": "2.0",
  "method": "eth_getCode",
  "params": ["0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48", "pending"],
  "id": 1
}
```

**Response**

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": "0x608060405234801561001057600080fd5b50..."
}
```

## Behaviour at `pending` on this node

> **Not available yet.** Today `pending` returns the latest confirmed block for this method, so it
> will not show changes made by transactions that are only pre-confirmed. The behaviour described
> below is planned.

Once available, `pending` will show a contract deployed by a pre-confirmed transaction, before its
block is sealed.

The bytecode returned is the account's original bytecode, not a padded representation.
