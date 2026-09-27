# eth_getProof

Returns the Merkle proof of an account and of a set of its storage slots, as defined in EIP-1186.

| | |
|---|---|
| Flashblocks `pending` | **Refused** — `pending` returns an error. `eth_getMultiProof` and `eth_getAccount` follow the same rule |

## Parameters

| Name | Type | Required | Description |
|---|---|---|---|
| `address` | string | yes | The 20-byte account to prove |
| `storageKeys` | array | yes | The storage slots to prove, as hex quantities. May be empty |
| `block` | string | no | Block number in hex, or a tag. Defaults to `"latest"` |

## Returns

`object` — the EIP-1186 account proof: `accountProof`, `balance`, `codeHash`, `nonce`,
`storageHash` and `storageProof`.

## Example

**Request**

```json
{
  "jsonrpc": "2.0",
  "method": "eth_getProof",
  "params": ["0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48", ["0x0"], "pending"],
  "id": 1
}
```

**Response**

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "error": {
    "code": -32602,
    "message": "pending is not supported by this method: pending state is built from flashblocks and carries no state root, so no proof can be derived from it"
  }
}
```

The same request with `"latest"` or a block number returns a normal proof.

## Behaviour at `pending` on this node

Pre-confirmed state is built from flashblocks and has no state root. A proof over it would carry
pending values in its leaf and the confirmed block's hashes in its branches, and it would verify
against no block on any chain. The node refuses the request instead of answering wrongly.

`eth_getMultiProof` and `eth_getAccount` need the same state root, so they refuse `pending` in the
same way.

A node that is not connected to a flashblocks producer has no pre-confirmed state, and answers
`pending` for these three methods as any reth node does.
