# eth_call

Executes a message call immediately, without broadcasting a transaction. Nothing is written to state.

| | |
|---|---|
| Flashblocks `pending` | **Yes** — `pending` reflects the flashblock being built |

## Parameters

| Name | Type | Required | Description |
|---|---|---|---|
| `transaction` | object | yes | The call. Fields: `from`, `to`, `gas`, `gasPrice`, `maxFeePerGas`, `maxPriorityFeePerGas`, `value`, `data` |
| `block` | string | no | Block number in hex, or a tag. Defaults to `"latest"` |
| `stateOverrides` | object | no | Per-account state overrides |
| `blockOverrides` | object | no | Block field overrides |

## Returns

`string` — the return value of the call, hex-encoded.

## Example

**Request**

```json
{
  "jsonrpc": "2.0",
  "method": "eth_call",
  "params": [
    {
      "to": "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48",
      "data": "0x70a082310000000000000000000000004200000000000000000000000000000000000006"
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
  "result": "0x0000000000000000000000000000000000000000000000000000000005f5e100"
}
```

## Behaviour at `pending` on this node

`pending` executes against the flashblock being built, so the call sees state left by
pre-confirmed transactions the builder has already sequenced.

This node supports two extra parameters that the standard JSON-RPC shape does not require:
`stateOverrides` and `blockOverrides`. Both are honoured at `pending`.

## Errors

| Code | Message |
|---|---|
| `-32000` | `execution reverted` — the revert reason, where one exists, is in the error `data` |
