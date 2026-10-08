# Read the state that pre-confirmed transactions left

Example 4 of 7 in [Examples](./README.md). The commands use `rpc` and the variables of
[Before you start](./README.md#before-you-start). Run them while the two transactions of
[Send a transaction and wait for its pre-confirmation](./send-and-wait.md) are pre-confirmed.

Here they ran before block `0x5d` was sealed. Each pair asks `latest`, then `pending`.

```sh
rpc eth_getBalance "[\"$TO\", \"latest\"]"
rpc eth_getBalance "[\"$TO\", \"pending\"]"
```

```json
{ "jsonrpc": "2.0", "id": 1, "result": "0x0" }
{ "jsonrpc": "2.0", "id": 1, "result": "0xde0b6b3a7640000" }
```

```sh
rpc eth_getTransactionCount "[\"$FROM\", \"latest\"]"
rpc eth_getTransactionCount "[\"$FROM\", \"pending\"]"
```

```json
{ "jsonrpc": "2.0", "id": 1, "result": "0x5" }
{ "jsonrpc": "2.0", "id": 1, "result": "0x7" }
```

```sh
rpc eth_getAccountInfo "[\"$TO\", \"pending\"]"
```

```json
{ "jsonrpc": "2.0", "id": 1, "result": { "balance": "0xde0b6b3a7640000", "nonce": "0x0", "code": "0x" } }
```

```sh
rpc eth_call "[{\"to\": \"$COUNTER\", \"data\": \"0x8381f58a\"}, \"latest\"]"
rpc eth_call "[{\"to\": \"$COUNTER\", \"data\": \"0x8381f58a\"}, \"pending\"]"
```

```json
{ "jsonrpc": "2.0", "id": 1, "result": "0x0000000000000000000000000000000000000000000000000000000000000002" }
{ "jsonrpc": "2.0", "id": 1, "result": "0x0000000000000000000000000000000000000000000000000000000000000003" }
```

`pending` includes the pre-confirmed transactions and `latest` does not: the new address holds
1 ETH, the sender's count is two higher, and the counter's `number()` (selector `0x8381f58a`) is
one higher. The `pending` count also counts this node's mempool, so use it as the next nonce;
`eth_getAccountInfo` returns the executed nonce only. `eth_call` at `pending` runs in the pending
block's environment. See [`eth_getBalance`](../ethereum-json-rpc-api/eth_getBalance.md),
[`eth_getTransactionCount`](../ethereum-json-rpc-api/eth_getTransactionCount.md) and
[`eth_call`](../ethereum-json-rpc-api/eth_call.md).

---

Next: [Simulate at `pending`](./simulate.md).
