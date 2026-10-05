# eth_getProof

Returns the Merkle proof of an account and of a set of its storage slots, as defined in EIP-1186.

| | |
|---|---|
| Flashblocks `pending` | **Refused** — `pending` returns an error while it is the flashblock being built. `eth_getMultiProof` and `eth_getAccount` follow the same rule |

## Parameters

| Name | Type | Required | Description |
|---|---|---|---|
| `address` | string | yes | The 20-byte account to prove |
| `storageKeys` | array | yes | The storage slots to prove, as hex quantities. May be empty. At most 1024; more is refused |
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

The same request with `"latest"` returns a normal proof. A block number or hash returns one only
within `--rpc.eth-proof-window` blocks of the head, 0 by default; further back the node answers
`-32602` and `distance to target block exceeds maximum proof window`. `eth_getMultiProof` and
`eth_getAccount` follow the same rule.

## Behaviour at `pending` on this node

Pre-confirmed state is built from flashblocks and has no state root. A proof over it would carry
pending values in its leaf and the confirmed block's hashes in its branches, and it would verify
against no block on any chain. The node refuses the request instead of answering wrongly.

`eth_getMultiProof` and `eth_getAccount` need the same state root, so they refuse `pending` in the
same way.

The refusal holds only while `pending` is the flashblock being built. When the node has executed
the next block for its consensus client, and that block is at least as new as the flashblock,
`pending` is that executed block. It has a state root, so the three methods answer from it.

A node that is not connected to a flashblocks producer has no pre-confirmed state, and answers
`pending` for these three methods as any reth node does. A connected node does the same while it
holds no flashblock for the next block: in a slot without flashblocks, and before a slot's first
flashblock. In both cases reth answers with `-32001` and the message `block not found: pending`,
except while its engine holds an executed block that is not yet canonical.

## At most 1024 storage keys

A request with more than 1024 storage keys is refused with `-32602` and the message
`too many storage keys: max 1024, got N`, before any trie work. Each key is a Merkle trie walk,
and an unbounded request could hold the node's executor for seconds. The cap is go-ethereum's.
`eth_getMultiProof` takes the same cap over all its accounts together: more than 1024 storage
keys in total are refused the same way.
