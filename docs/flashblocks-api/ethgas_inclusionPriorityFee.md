# ethgas_inclusionPriorityFee

Returns the builder's inclusion fee from the latest flashblock, with where the number came from and
how old it is.

| | |
|---|---|
| At `pending` | No block parameter |
| Transport | HTTP, WebSocket or IPC. Available only with `--flashblocks-url`, on every configured transport, whatever `--http.api` says |

## Parameters

None.

## Returns

`object`

| Field | Type | Description |
|---|---|---|
| `maxPriorityFeePerGas` | hex quantity | The priority fee per gas, in wei |
| `source` | string | `"builder"` while the builder's value is fresh, `"fallback"` otherwise |
| `ageMs` | integer | Milliseconds since the flashblock that carried the value arrived. Builder only |
| `blockNumber` | integer | The block of that flashblock. Builder only |
| `flashblockIndex` | integer | The index of that flashblock. Builder only |

## Example

**Request**

```json
{
  "jsonrpc": "2.0",
  "method": "ethgas_inclusionPriorityFee",
  "params": [],
  "id": 1
}
```

**Response, builder value**

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "maxPriorityFeePerGas": "0x4190ab00",
    "source": "builder",
    "ageMs": 412,
    "blockNumber": 2293195,
    "flashblockIndex": 3
  }
}
```

**Response, fallback**

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "maxPriorityFeePerGas": "0x3b9aca00",
    "source": "fallback"
  }
}
```

## Behaviour on this node

The node holds the fee of the latest flashblock it received, and serves it while three things
hold: its block is not yet canonical on this node, it is at most `--flashblocks.fee-max-age` old
(15 s by default), and it is at most `--flashblocks.fee-ceiling` (1,000 gwei by default).
Otherwise the node serves reth's own suggestion, marked `"fallback"`, without the three builder
fields. So after the fee's block is sealed, a slot that another builder builds gets the fallback.

A flashblock without a fee clears the held fee. The node takes the fee when the flashblock arrives,
also from a flashblock that it rejects later.

The same number reaches three standard methods:

- `eth_maxPriorityFeePerGas` returns `maxPriorityFeePerGas` as a bare quantity.
- `eth_gasPrice` returns the next block's base fee plus `maxPriorityFeePerGas` while the builder's
  value is served, and reth's own gas price otherwise.
- `eth_fillTransaction` uses it as the priority fee of a request that sets no fee, and twice the
  latest block's base fee plus it as the maximum fee.

Clients that estimate from `eth_feeHistory`, such as alloy's default estimator, do not see it.
