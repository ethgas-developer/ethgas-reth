# eth_sendRawTransactionSync

Submits a signed transaction and waits until a flashblock or a sealed block holds it, then returns
its receipt.

| | |
|---|---|
| At `pending` | No block parameter |
| Transport | HTTP, WebSocket or IPC |

This is the method to use when you want pre-confirmation rather than a transaction hash. It saves
you submitting with `eth_sendRawTransaction` and then polling
[`eth_getTransactionReceipt`](../ethereum-json-rpc-api/eth_getTransactionReceipt.md).

## Parameters

| Name | Type | Required | Description |
|---|---|---|---|
| `data` | string | yes | The signed transaction, RLP-encoded as a hex string |
| `timeout_ms` | integer | no | How long to wait, in milliseconds, as a JSON number (a hex string is refused with `-32602`). The node reduces a larger value to its configured maximum, `--rpc.send-raw-transaction-sync-timeout`, 30 s unless the operator sets it, and never refuses it. `0` or absent waits that maximum |

## Returns

A receipt object, in the same shape
[`eth_getTransactionReceipt`](../ethereum-json-rpc-api/eth_getTransactionReceipt.md) returns.

## Example

**Request**

```json
{
  "jsonrpc": "2.0",
  "method": "eth_sendRawTransactionSync",
  "params": ["0x02f8…", 3000],
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

## What the call waits for

The call returns the first receipt it finds: in the current pre-confirmed block, in a later
flashblock, or in a sealed block. A receipt from a flashblock carries `blockHash: 0x000…0`; a
receipt from a sealed block carries that block's hash.

If no flashblock and no sealed block holds the transaction before the timeout, the call fails with
code `4`. The message names the transaction hash and the duration waited, which is the reduced
value. The transaction stays in the mempool and can still land.

## The receipt is not proof of inclusion

A receipt with `blockHash: 0x000…0` means that the builder has sequenced your transaction, not that
it is in a sealed block.

**To confirm inclusion**, poll
[`eth_getTransactionReceipt`](../ethereum-json-rpc-api/eth_getTransactionReceipt.md) until
`blockHash` is non-zero. If another builder's block wins the slot, the transaction is sealed at
another position, possibly with another status, or not at all, and the receipt changes.

## Errors

The node refuses a transaction at once, before it waits, with the errors of
[`eth_sendRawTransaction`](../ethereum-json-rpc-api/eth_sendRawTransaction.md#errors). After the
wait it can fail with code `4` (above).
