# Price a transaction for the next flashblock

Example 2 of 7 in [Examples](./README.md). The commands use `rpc` and the variables of
[Before you start](./README.md#before-you-start).

```sh
rpc ethgas_inclusionPriorityFee
rpc eth_maxPriorityFeePerGas
```

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "maxPriorityFeePerGas": "0x1ad27480",
    "source": "builder",
    "ageMs": 789,
    "blockNumber": 93,
    "flashblockIndex": 2
  }
}
```

```json
{ "jsonrpc": "2.0", "id": 1, "result": "0x1ad27480" }
```

The builder asks a priority fee of 0.45 gwei (`0x1ad27480` wei) to admit a transaction into the
next flashblock. The price came in flashblock 2 of block 93, which arrived 789 ms before the call.
`eth_maxPriorityFeePerGas` serves the same number, so a library that reads it pays the builder's
price. A transaction that pays less still lands in the same block, in its last flashblock. When
the builder sends no fee, `source` is `"fallback"` and the number is reth's own suggestion. See
[`ethgas_inclusionPriorityFee`](../flashblocks-api/ethgas_inclusionPriorityFee.md).

`eth_fillTransaction` fills a transaction around that fee:

```sh
rpc eth_fillTransaction "[{\"from\": \"$FROM\", \"to\": \"$TO\", \"value\": \"0xde0b6b3a7640000\"}]"
```

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "raw": "0x02f58330182405841ad27480841afc1c6c825208…c0808080",
    "tx": {
      "type": "0x2",
      "chainId": "0x301824",
      "nonce": "0x5",
      "gas": "0x5208",
      "maxFeePerGas": "0x1afc1c6c",
      "maxPriorityFeePerGas": "0x1ad27480",
      "to": "0xca748b2b45f964df8ba80747da0757cb76dbb76c",
      "value": "0xde0b6b3a7640000"
    }
  }
}
```

The priority fee is the builder's. `maxFeePerGas` is twice the latest block's base fee plus that
fee. The nonce is the `pending` transaction count, and the gas is estimated at `pending`. `raw`
encodes `tx` with a zero signature: sign `tx`, then send it. See the `eth_fillTransaction` row
of the [RPC overview](../rpc-overview.md#ethereum-json-rpc-api).

---

Next: [Send a transaction and wait for its pre-confirmation](./send-and-wait.md).
