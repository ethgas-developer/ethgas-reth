# ethgas_inclusionPriorityFee

Returns the priority fee that clears the builder's inclusion gate for the next flashblock, with
where the number came from and how old it is.

| | |
|---|---|
| Flashblocks `pending` | n/a — this method takes no block parameter |
| Transport | HTTP, WebSocket or IPC. Available only with `--flashblocks-url`, on every configured transport, whatever `--http.api` says |

## Parameters

None.

## Returns

`object`

| Field | Type | Description |
|---|---|---|
| `maxPriorityFeePerGas` | hex quantity | The fee to pay, in wei |
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

The builder publishes the fee in every flashblock it seals. The node keeps the latest one and
serves it while three things hold: its block is not yet sealed, it is at most
`--flashblocks.fee-max-age` old (15 s by default), and it is at most `--flashblocks.fee-ceiling`
(1,000 gwei by default). Otherwise, or when the builder published none, the node serves reth's own
suggestion, marked `"fallback"`, without the three builder fields. So after the builder's block is
sealed, a slot that another builder builds gets the fallback. reth's suggestion can sit near zero on this chain, because the last
flashblock of a slot admits every transaction whatever its tip.

The same number reaches three standard methods:

- `eth_maxPriorityFeePerGas` returns `maxPriorityFeePerGas` as a bare quantity.
- `eth_gasPrice` returns the next block's base fee plus `maxPriorityFeePerGas` while the builder's
  value is served, and reth's own gas price otherwise.
- `eth_fillTransaction` uses it as the priority fee of a request that sets no fee, and twice the
  base fee plus it as the maximum fee.

Wallets that estimate from `eth_feeHistory`, such as alloy's default estimator, or from their own
fee service, such as MetaMask, do not see it.

A transaction that pays less than this fee still lands in the same block, in its last flashblock,
because that flashblock admits every transaction. This is the price of the next flashblock, not
the price of the block.
