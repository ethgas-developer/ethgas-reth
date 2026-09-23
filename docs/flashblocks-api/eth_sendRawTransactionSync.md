# eth_sendRawTransactionSync

Submits a signed transaction and waits for it to be pre-confirmed into a flashblock, then returns
its receipt.

| | |
|---|---|
| Flashblocks `pending` | n/a — this method takes no block parameter |
| Transport | HTTP or WebSocket |

This is the method to use when you want pre-confirmation rather than a transaction hash. It saves
you submitting with `eth_sendRawTransaction` and then polling
[`eth_getTransactionReceipt`](../ethereum-json-rpc-api/eth_getTransactionReceipt.md).

## Parameters

| Name | Type | Required | Description |
|---|---|---|---|
| `data` | string | yes | The signed transaction, RLP-encoded as a hex string |
| `timeout_ms` | integer | no | How long to wait, in milliseconds. Defaults to `6000`, which is also the maximum |

## Returns

A receipt object, in the same shape
[`eth_getTransactionReceipt`](../ethereum-json-rpc-api/eth_getTransactionReceipt.md) returns.

## Example

**Request**

```json
{
  "jsonrpc": "2.0",
  "method": "eth_sendRawTransactionSync",
  "params": ["0x02f8710182...", 3000],
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

## The timeout is a hard limit, not a clamp

`timeout_ms` above **6000** is rejected outright with `-32602` and the message
`time out too long`. It is not silently reduced. A client that retries blindly on error can spin
hard against this, so treat the error as fatal rather than retryable.

If the transaction misses its flashblock window, the call waits on confirmation instead, which
cannot land inside the timeout. Expect a timeout error in that case, not a receipt.

## The receipt is not proof of inclusion

The receipt returned carries `blockHash: 0x000…0`, because a flashblock is not sealed. It means the
builder has sequenced your transaction, not that it is in a confirmed block.

**To confirm inclusion**, poll
[`eth_getTransactionReceipt`](../ethereum-json-rpc-api/eth_getTransactionReceipt.md) until
`blockHash` is non-zero.

## Errors

| Code | Message |
|---|---|
| `-32602` | `time out too long` — `timeout_ms` above 6000 |
| `-32000` | `nonce too low` |
| `-32000` | `insufficient funds for gas * price + value` |
| `-32000` | `already known` |
| `-32000` | `replacement transaction underpriced` |
