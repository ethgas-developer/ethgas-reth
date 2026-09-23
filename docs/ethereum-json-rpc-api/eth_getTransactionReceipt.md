# eth_getTransactionReceipt

Returns the receipt of a transaction.

| | |
|---|---|
| Flashblocks `pending` | **Yes** — `pending` reflects the flashblock being built |

## Parameters

| Name | Type | Required | Description |
|---|---|---|---|
| `transactionHash` | string | yes | The 32-byte transaction hash |

## Returns

`object | null` — a receipt object, or `null` if the transaction is unknown. Fields include
`transactionHash`, `transactionIndex`, `blockHash`, `blockNumber`, `from`, `to`,
`cumulativeGasUsed`, `effectiveGasPrice`, `gasUsed`, `contractAddress`, `logs`, `logsBloom`,
`type` and `status`.

## Example

**Request**

```json
{
  "jsonrpc": "2.0",
  "method": "eth_getTransactionReceipt",
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
    "transactionHash": "0x7f4e2a8c1b6d9035e4a7c2f8b1d6e9a34c7f0b5d8e2a6c9f3b7d1e4a8c2f6b0c3",
    "transactionIndex": "0x0",
    "blockHash": "0x0000000000000000000000000000000000000000000000000000000000000000",
    "blockNumber": "0x10f2c5",
    "from": "0xd3CdA913deB6f4967b2Ef66ae97DE114a83bcc01",
    "to": "0x742d35Cc6634C0532925a3b8D4C9dD0b4f3BaEa",
    "cumulativeGasUsed": "0x5208",
    "effectiveGasPrice": "0x4c4b40",
    "gasUsed": "0x5208",
    "contractAddress": null,
    "logs": [],
    "logsBloom": "0x0000...0000",
    "status": "0x1",
    "type": "0x2"
  }
}
```

## Behaviour at `pending` on this node

Confirmed receipts are checked first, then pre-confirmed flashblock data, so a receipt is
available before the block is sealed.

**This is the most dangerous method on the node to misread.**

A pre-confirmed receipt carries `blockHash: 0x000…0`. It looks exactly like a normal receipt
otherwise: `status: "0x1"`, a real `gasUsed`, real logs. It is **not** proof that the transaction
is in a sealed block. The builder sequenced it; the block has not been produced yet.

Many nodes return `null` for a transaction that is not yet mined, so a caller written against that
behaviour may treat any receipt it gets back as proof of inclusion. On this node that assumption is
wrong.

**To confirm inclusion**, check that `blockHash` is non-zero.

## Errors

| Code | Message |
|---|---|
| `-32000` | `transaction indexing is in progress` — the node is still syncing |
