# eth_getTransactionReceipt

Returns the receipt of a transaction.

| | |
|---|---|
| At `pending` | No block parameter. Also returns pre-confirmed receipts |

## Parameters

| Name | Type | Required | Description |
|---|---|---|---|
| `transactionHash` | string | yes | The 32-byte transaction hash |

## Returns

`object | null`: a receipt object, or `null` when no sealed block and no pre-confirmed block holds
the transaction. A transaction that only the mempool holds gives `null`. Fields include
`transactionHash`, `transactionIndex`, `blockHash`, `blockNumber`, `from`, `to`,
`cumulativeGasUsed`, `effectiveGasPrice`, `gasUsed`, `contractAddress`, `logs`, `logsBloom`,
`type` and `status`.

## Example

**Request**

```json
{
  "jsonrpc": "2.0",
  "method": "eth_getTransactionReceipt",
  "params": ["0x7f4e2a8c1b6d9035e4a7c2f8b1d6e9a34c7f0b5d8e2a6c9f3b7d1e4a8c2f6b0c"],
  "id": 1
}
```

**Response**

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "transactionHash": "0x7f4e2a8c1b6d9035e4a7c2f8b1d6e9a34c7f0b5d8e2a6c9f3b7d1e4a8c2f6b0c",
    "transactionIndex": "0x0",
    "blockHash": "0x0000000000000000000000000000000000000000000000000000000000000000",
    "blockNumber": "0x10f2c5",
    "from": "0xd3cda913deb6f4967b2ef66ae97de114a83bcc01",
    "to": "0x742d35cc6634c0532925a3b8d4c9dd0b4f3baea1",
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

Sealed receipts are checked first, then the pre-confirmed block, so a receipt is available before
the block is sealed.

A pre-confirmed receipt carries `blockHash: 0x000…0`, and so do its logs. Otherwise it looks like a
sealed receipt: a real `status`, which can be `0x0`, a real `gasUsed` and real logs. It is **not**
proof that the transaction is in a sealed block. The builder sequenced it; this node has not
imported a sealed block that holds it.

Many nodes return `null` for a transaction that is not yet mined, so a caller written against that
behaviour may treat any receipt it gets back as proof of inclusion. On this node that assumption is
wrong.

**To confirm inclusion**, check that `blockHash` is non-zero.

**A pre-confirmed receipt can change, or vanish.** If another builder's block wins the slot (on the
test network between about 1 block in 30 and 1 in 800), the transaction is sealed in that block at
another position, with other log indexes and possibly another status, or it is not sealed at all
when a transaction with the same nonce takes its place. Its receipt then changes, or becomes
`null`. The receipt also becomes `null` when the builder starts the block again without the
transaction, and for the rest of a block after the node rejects one of its flashblocks. That `null`
does not mean that the transaction is dropped: it can still land in the sealed block.

Several clients treat any receipt as final: web3.py's `wait_for_transaction_receipt`, alloy's
`get_receipt`, viem's `waitForTransactionReceipt`, and ethers' `tx.wait()` when its first fetch
finds a pre-confirmed receipt. viem's `waitForTransactionReceipt` with `confirmations` of 2 or more
keeps the pre-confirmed receipt it fetched first.
