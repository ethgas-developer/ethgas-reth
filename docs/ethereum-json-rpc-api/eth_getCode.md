# eth_getCode

Returns the contract bytecode deployed at an address.

| | |
|---|---|
| Flashblocks `pending` | **Yes** |

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

`pending` shows a contract deployed by a pre-confirmed transaction, before its block is sealed. The
code comes from the node's own execution of the flashblock transactions on top of the latest
confirmed block, so it agrees with what `eth_call` at `pending` runs against.

A contract that exists only in the node's mempool is never visible at `pending`.

The bytecode returned is the account's original bytecode, not a padded representation.
