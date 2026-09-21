# eth_getTransactionCount

Returns the number of transactions sent from an address, that is, the next nonce.

| | |
|---|---|
| Flashblocks `pending` | **Yes** — `pending` reflects the flashblock being built |

## Parameters

| Name | Type | Required | Description |
|---|---|---|---|
| `address` | string | yes | The 20-byte address to query |
| `block` | string | no | Block number in hex, or a tag. Defaults to `"latest"` |

## Returns

`string` — the transaction count as a hex quantity.

## Example

**Request**

```json
{
  "jsonrpc": "2.0",
  "method": "eth_getTransactionCount",
  "params": ["0x742d35Cc6634C0532925a3b8D4C9dD0b4f3BaEa", "pending"],
  "id": 1
}
```

**Response**

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": "0x4d2"
}
```

## Behaviour at `pending` on this node

`pending` counts transactions the builder has already sequenced into flashblocks, so it is the
nonce to use when submitting back-to-back transactions faster than the block time.

This differs from a typical node, where `pending` counts transactions sitting in the node's own
mempool. Here it reflects what the builder actually sequenced.
