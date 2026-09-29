# eth_getTransactionByHash

Returns a transaction by its hash.

| | |
|---|---|
| Flashblocks `pending` | **Yes** — `pending` reflects the flashblock being built |

## Parameters

| Name | Type | Required | Description |
|---|---|---|---|
| `transactionHash` | string | yes | The 32-byte transaction hash |

## Returns

`object | null` — a transaction object, or `null` if the transaction is unknown.

## Example

**Request**

```json
{
  "jsonrpc": "2.0",
  "method": "eth_getTransactionByHash",
  "params": ["0x7f4e2a8c1b6d9035e4a7c2f8b1d6e9a34c7f0b5d8e2a6c9f3b7d1e4a8c2f6b0c3"],
  "id": 1
}
```

**Response**

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "hash": "0x7f4e2a8c1b6d9035e4a7c2f8b1d6e9a34c7f0b5d8e2a6c9f3b7d1e4a8c2f6b0c3",
    "blockHash": null,
    "blockNumber": "0x10f2c5",
    "transactionIndex": "0x0",
    "from": "0xd3CdA913deB6f4967b2Ef66ae97DE114a83bcc01",
    "to": "0x742d35Cc6634C0532925a3b8D4C9dD0b4f3BaEa",
    "value": "0x2c68af0bb14000",
    "gas": "0x5208",
    "maxFeePerGas": "0x4c4b40",
    "maxPriorityFeePerGas": "0xf4240",
    "nonce": "0x4d2",
    "input": "0x",
    "type": "0x2",
    "chainId": "0x88bb0"
  }
}
```

## Behaviour at `pending` on this node

Three sources are checked, in this order: the sealed chain, this node's mempool, and the flashblock
being built. The first one that knows the hash answers.

- **Sealed.** `blockHash` is non-null. This is the only signal that the transaction is in a block.
- **In this node's mempool.** `blockHash`, `blockNumber` and `transactionIndex` are all `null`, and
  they stay `null` after the builder sequences the transaction, until its block is sealed. Every
  transaction sent through this node with `eth_sendRawTransaction` or `eth_sendRawTransactionSync`
  is in its mempool, and so is every transaction a peer gossiped to it.
- **In the flashblock only.** `blockHash` is `null` while `blockNumber` and `transactionIndex` are
  set, as in the example above. This is how a transaction looks that this node's mempool has never
  held.

So a `null` `blockNumber` does not mean the builder has not sequenced the transaction. To detect
pre-confirmation, call [`eth_getTransactionReceipt`](./eth_getTransactionReceipt.md), which returns
the pre-confirmed receipt in both pending cases, or submit with
[`eth_sendRawTransactionSync`](../flashblocks-api/eth_sendRawTransactionSync.md) and wait for it.

This method takes no block parameter, but it is flashblocks-aware regardless.
