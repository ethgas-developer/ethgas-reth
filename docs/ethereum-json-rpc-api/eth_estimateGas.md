# eth_estimateGas

Estimates the gas a transaction would consume.

| | |
|---|---|
| Flashblocks `pending` | **Yes** — `pending` reflects the flashblock being built |

## Parameters

| Name | Type | Required | Description |
|---|---|---|---|
| `transaction` | object | yes | The transaction. Fields: `from`, `to`, `gas`, `gasPrice`, `maxFeePerGas`, `maxPriorityFeePerGas`, `value`, `data` |
| `block` | string | no | Block number in hex, or a tag. Defaults to `"latest"` |
| `stateOverrides` | object | no | Per-account state overrides |
| `blockOverrides` | object | no | Block field overrides |

## Returns

`string` — the estimated gas as a hex quantity.

## Example

**Request**

```json
{
  "jsonrpc": "2.0",
  "method": "eth_estimateGas",
  "params": [
    {
      "from": "0xd3CdA913deB6f4967b2Ef66ae97DE114a83bcc01",
      "to": "0x742d35Cc6634C0532925a3b8D4C9dD0b4f3BaEa",
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

The estimate runs against the flashblock being built, so it accounts for state changed by
pre-confirmed transactions.

## Errors

| Code | Message |
|---|---|
| `-32000` | `execution reverted` |
