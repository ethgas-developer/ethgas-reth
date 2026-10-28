# eth_estimateGas

Estimates the gas a transaction would consume.

| | |
|---|---|
| At `pending` | **Pre-confirmed** |

## Parameters

| Name | Type | Required | Description |
|---|---|---|---|
| `transaction` | object | yes | The transaction. Fields: `from`, `to`, `gas`, `gasPrice`, `maxFeePerGas`, `maxPriorityFeePerGas`, `value`, `data` |
| `block` | string | no | Block number in hex, or a tag. Defaults to `"pending"` while flashblock data is served, else `"latest"` |
| `stateOverrides` | object | no | Per-account state overrides |
| `blockOverrides` | object | no | Block field overrides |

## Returns

`string`: the estimated gas as a hex quantity.

## Example

**Request**

```json
{
  "jsonrpc": "2.0",
  "method": "eth_estimateGas",
  "params": [
    {
      "from": "0xd3cda913deb6f4967b2ef66ae97de114a83bcc01",
      "to": "0x742d35cc6634c0532925a3b8d4c9dd0b4f3baea1",
      "value": "0x2c68af0bb14000"
    },
    "pending"
  ],
  "id": 1
}
```

**Response**

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": "0x5208"
}
```

## Behaviour at `pending` on this node

The estimate runs against the pre-confirmed block, so it accounts for state changed by
pre-confirmed transactions. It runs in the pre-confirmed block's own environment: `block.number`,
`block.timestamp` and the base fee are those of the block being built, and after the Amsterdam
fork so is the slot number that `SLOTNUM` reads.

**Without a block parameter**, the estimate runs at `pending` while flashblock data is served, so
it sees the pre-confirmed transactions before it, such as an approval. Without flashblock data it
runs at `latest`.

## Errors

| Code | Message |
|---|---|
| `3` | `execution reverted`; the revert data, where one exists, is in the error `data` |
