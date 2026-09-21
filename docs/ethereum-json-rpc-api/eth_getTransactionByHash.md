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

Confirmed data is checked first, then pre-confirmed flashblock data, so a pre-confirmed
transaction is visible here before its block is sealed.

**A pending transaction has `blockHash: null`** while it carries a real `blockNumber`. That pairing
is how you tell a pre-confirmed transaction from a mined one. Treat a non-null `blockHash` as the
signal that the transaction is in a sealed block.

This method takes no block parameter, but it is flashblocks-aware regardless.
