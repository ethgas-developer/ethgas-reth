# eth_sendRawTransaction

Submits a signed transaction to the node.

| | |
|---|---|
| Flashblocks `pending` | n/a — this method takes no block parameter |

## Parameters

| Name | Type | Required | Description |
|---|---|---|---|
| `data` | string | yes | The signed transaction, RLP-encoded as a hex string |

## Returns

`string` — the 32-byte transaction hash, if the transaction was accepted.

## Example

**Request**

```json
{
  "jsonrpc": "2.0",
  "method": "eth_sendRawTransaction",
  "params": ["0x02f86b82210501843b9aca008477359400825208944200000000000000000000000000000000000006872c68af0bb1400080c001a0..."],
  "id": 1
}
```

**Response**

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": "0x7f4e2a8c1b6d9035e4a7c2f8b1d6e9a34c7f0b5d8e2a6c9f3b7d1e4a8c2f6b0c3"
}
```

## Behaviour at `pending` on this node

This method takes no block parameter. It returns as soon as the transaction is accepted, and the
hash it returns says nothing about inclusion.

**To submit and wait for pre-confirmation in one call, use `eth_sendRawTransactionSync`.** It
returns the receipt once the transaction appears in a flashblock, with a default and maximum
timeout of 6000 ms.

## Errors

| Code | Message |
|---|---|
| `-32000` | `nonce too low` |
| `-32000` | `insufficient funds for gas * price + value` |
| `-32000` | `already known` |
| `-32000` | `replacement transaction underpriced` |
