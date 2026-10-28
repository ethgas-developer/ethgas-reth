# eth_call

Executes a message call immediately, without broadcasting a transaction. Nothing is written to state.

| | |
|---|---|
| At `pending` | **Pre-confirmed** |

## Parameters

| Name | Type | Required | Description |
|---|---|---|---|
| `transaction` | object | yes | The call. Fields: `from`, `to`, `gas`, `gasPrice`, `maxFeePerGas`, `maxPriorityFeePerGas`, `value`, `data` |
| `block` | string | no | Block number in hex, or a tag. Defaults to `"latest"` |
| `stateOverrides` | object | no | Per-account state overrides |
| `blockOverrides` | object | no | Block field overrides |

## Returns

`string`: the return value of the call, hex-encoded.

## Example

**Request**

```json
{
  "jsonrpc": "2.0",
  "method": "eth_call",
  "params": [
    {
      "to": "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48",
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

`pending` executes against the pre-confirmed block, so the call sees state left by pre-confirmed
transactions the builder has already sequenced.

The block environment is the pre-confirmed block's own: `block.number`, `block.timestamp`, the base
fee, `COINBASE`, `PREVRANDAO` and the gas limit are those of the block being built, and after the
Amsterdam fork so is the slot number that `SLOTNUM` reads. At any block tag, a call whose gas price
works out to 0 (no fee field, a `gasPrice` of 0, or a `maxFeePerGas` of 0) runs with a base fee of
0, so `BASEFEE` reads 0; set `gasPrice`, `maxFeePerGas` or `maxPriorityFeePerGas` to read the
block's base fee. A state override applies on top of the pre-confirmed state, so overriding one
field of an account keeps the account's other fields as the pre-confirmed block left them.

When the node imports a block, `pending` is that block if the node has no flashblock data or the
block is at least as new as the pre-confirmed block. When the node has no flashblock data and
imports no block, `pending` is the latest block's state in the environment of the block that would
follow it.

The environment and the state are read one after the other; see
[At a block boundary](../rpc-overview.md#at-a-block-boundary). Both overrides are honoured at
`pending`.

## Errors

| Code | Message |
|---|---|
| `3` | `execution reverted`; the revert data, where one exists, is in the error `data` |
