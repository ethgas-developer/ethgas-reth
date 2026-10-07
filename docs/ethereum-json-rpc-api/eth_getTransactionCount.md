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
into flashblocks, raised past the account's transactions that this node's mempool holds and that
follow it. So two transactions sent through this node before the first is pre-confirmed get two
nonces.

The flashblock part is the node's own execution of the flashblock transactions. So it also covers
the two ways a nonce rises without a transaction from the address: an EIP-7702 authorization the
address signed, and a contract creation by a contract, which raises that contract's own nonce.

`eth_getAccountInfo` at `pending` returns the executed nonce only, so the two differ by the
mempool's transactions. A transaction sent through another node is counted once it is in a
flashblock; track nonces locally if you send through several nodes. `eth_fillTransaction` fills a
missing nonce with this value.
