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

`pending` returns the account's nonce after the transactions the builder has already sequenced
into flashblocks, so it is the nonce to use when submitting back-to-back transactions faster than
the block time.

This differs from a typical node, where `pending` counts transactions sitting in the node's own
mempool. Here it reflects what the builder actually sequenced, and the mempool is never counted.

The node executes the flashblock transactions and returns the executed nonce. So it also covers
the two ways a nonce rises without a transaction from the address: an EIP-7702 authorization the
address signed, and a contract creation by a contract, which raises that contract's own nonce. It
always agrees with the nonce that `eth_getAccountInfo` returns at `pending`.
