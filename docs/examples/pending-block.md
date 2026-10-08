# Read the pre-confirmed block

Example 1 of 7 in [Examples](./README.md). The commands use `rpc` and the variables of
[Before you start](./README.md#before-you-start).

```sh
rpc eth_blockNumber
rpc eth_getBlockByNumber '["pending", false]'
```

```json
{ "jsonrpc": "2.0", "id": 1, "result": "0x5c" }
```

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "number": "0x5d",
    "hash": "0x0000000000000000000000000000000000000000000000000000000000000000",
    "parentHash": "0x9a182136ea972347ac3048d75f0b206f0c2f75ac81a9eba89074cd623e097c61",
    "timestamp": "0x6ac60fc4",
    "gasUsed": "0x7a1eba",
    "baseFeePerGas": "0x136bf5",
    "transactions": [
      "0xffbcb21315bd4b2bbaf3a5c9331a04c65e5dfdb8e847e8f867e25a021cabc66c",
      "0xfe840a953984cc8425753ac2bdbeabd6982f5f05c146b78042f79969e8318278"
    ]
  }
}
```

`pending` is the block the builder is assembling: its `number` is the latest block's plus one, and
its `parentHash` is the latest block's hash. Its `hash` is `0x000…0` because the block is not
sealed; do not pass it to `eth_getBlockByHash`. `transactions` held 176 hashes here, and the list
grows with each flashblock until the block is sealed. Without flashblock data, `pending` is a block
the node executed, never one assembled from its mempool. See
[`eth_getBlockByNumber`](../ethereum-json-rpc-api/eth_getBlockByNumber.md).

---

Next: [Price a transaction for the next flashblock](./price.md).
