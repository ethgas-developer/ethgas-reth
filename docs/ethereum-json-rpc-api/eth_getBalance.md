# eth_getBalance

Returns the ETH balance of an account.

| | |
|---|---|
| At `pending` | **Pre-confirmed** |

## Parameters

| Name | Type | Required | Description |
|---|---|---|---|
| `address` | string | yes | The 20-byte address to query |
| `block` | string | no | Block number in hex, or a tag. Defaults to `"latest"` |

## Returns

`string`: the balance in wei, as a hex quantity.

## Example

**Request**

```json
{
  "jsonrpc": "2.0",
  "method": "eth_getBalance",
  "params": ["0x742d35cc6634c0532925a3b8d4c9dd0b4f3baea1", "pending"],
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

1. **If the builder reported a balance for the address in any flashblock of the pre-confirmed
   block**, you get the latest balance it reported.
2. **Otherwise**, you get the balance from the node's own execution of the pre-confirmed
   transactions on top of the latest block.

Both are live. One difference: the node's own execution does not apply the pre-confirmed block's
withdrawals, so in case 2 a withdrawal credit appears only once the node imports the sealed block.
