# eth_getBalance

Returns the ETH balance of an account.

| | |
|---|---|
| Flashblocks `pending` | **Yes**, for balances the builder reports. **Not yet** for other addresses |

## Parameters

| Name | Type | Required | Description |
|---|---|---|---|
| `address` | string | yes | The 20-byte address to query |
| `block` | string | no | Block number in hex, or a tag. Defaults to `"latest"` |

## Returns

`string` — the balance in wei, as a hex quantity.

## Example

**Request**

```json
{
  "jsonrpc": "2.0",
  "method": "eth_getBalance",
  "params": ["0x742d35Cc6634C0532925a3b8D4C9dD0b4f3BaEa", "pending"],
  "id": 1
}
```

**Response**

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": "0x1a055690d9db80000"
}
```

## Behaviour at `pending` on this node

`pending` is answered in two ways, depending on the address.

1. **If the builder reported a new balance for the address in the current flashblock**, you get that
   balance. This is live, and it is the case that matters for an account transacting right now.
2. **If the address is not among those the builder touched**, you get its balance at the latest
   confirmed block.

> **Case 2 will improve.** It is planned that addresses the builder did not touch will also be
> answered from pre-confirmed state. Until then, treat a balance for an untouched address as being
> as of the latest confirmed block.
