# Send a transaction and wait for its pre-confirmation

Example 3 of 7 in [Examples](./README.md). The commands use `rpc` and the variables of
[Before you start](./README.md#before-you-start).

`eth_sendRawTransactionSync` answers once the transaction is in a flashblock:

```sh
TIP=$(rpc eth_maxPriorityFeePerGas | jq -r .result | cast to-dec)
NONCE=$(rpc eth_getTransactionCount "[\"$FROM\", \"pending\"]" | jq -r .result | cast to-dec)
RAW=$(cast mktx --rpc-url "$RPC" --private-key "$PRIVATE_KEY" --nonce "$NONCE" --priority-gas-price "$TIP" "$TO" --value 1ether)
rpc eth_sendRawTransactionSync "[\"$RAW\", 5000]"
```

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "type": "0x2",
    "status": "0x1",
    "cumulativeGasUsed": "0x8c942c",
    "logs": [],
    "transactionHash": "0x61629444d6748299582f4d985d90ad81394fc586aa1934615173daf45462c1fd",
    "transactionIndex": "0xca",
    "blockHash": "0x0000000000000000000000000000000000000000000000000000000000000000",
    "blockNumber": "0x5d",
    "gasUsed": "0x5208",
    "effectiveGasPrice": "0x1ae5e075",
    "from": "0xf61e98e7d47ab884c244e39e031978e33162ff4b",
    "to": "0xca748b2b45f964df8ba80747da0757cb76dbb76c",
    "contractAddress": null
  }
}
```

The four commands took 0.2 s. The receipt is pre-confirmed: `blockHash` is `0x000…0` and
`blockNumber` is the pending block. `5000` is the wait in milliseconds; the node clamps it to its
maximum, 30 s unless the operator sets it, and a timeout is error code `4`. The transaction pays
the builder's price, so it can enter the next flashblock. `cast mktx` takes its nonce from
`latest`, which leaves out pre-confirmed transactions, so the commands pass the `pending` count.
`cast to-dec` converts the node's hex answers: `cast mktx` refuses a hex priority fee. See
[`eth_sendRawTransactionSync`](../flashblocks-api/eth_sendRawTransactionSync.md).

Or send with `eth_sendRawTransaction`, and poll the receipt:

```sh
NONCE=$(rpc eth_getTransactionCount "[\"$FROM\", \"pending\"]" | jq -r .result | cast to-dec)
RAW=$(cast mktx --rpc-url "$RPC" --private-key "$PRIVATE_KEY" --nonce "$NONCE" --priority-gas-price "$TIP" "$COUNTER" "increment()")
HASH=$(rpc eth_sendRawTransaction "[\"$RAW\"]" | jq -r .result)
until rpc eth_getTransactionReceipt "[\"$HASH\"]" | jq -e .result > /dev/null; do sleep 0.2; done
rpc eth_getTransactionReceipt "[\"$HASH\"]"
```

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "type": "0x2",
    "status": "0x1",
    "cumulativeGasUsed": "0x9e0a0d",
    "logs": [],
    "transactionHash": "0xa4c188efeb01465e10ebd177a25a5df125f4532ffcf5bff8687b8984d24a73cb",
    "transactionIndex": "0xe4",
    "blockHash": "0x0000000000000000000000000000000000000000000000000000000000000000",
    "blockNumber": "0x5d",
    "gasUsed": "0x670e",
    "effectiveGasPrice": "0x1ae5e075",
    "from": "0xf61e98e7d47ab884c244e39e031978e33162ff4b",
    "to": "0xb7e811662fa10ac068aee115ac2e682821630535",
    "contractAddress": null
  }
}
```

The `pending` count already included the transfer above, so the second transaction got the next
nonce. Its receipt came about 1 s after the send, with `blockHash` `0x000…0`: it is pre-confirmed,
not sealed. **A pre-confirmed receipt can still change or vanish.** If another builder's block wins
the slot, the transaction is sealed at another position, possibly with another status, or not at
all. See [`eth_getTransactionReceipt`](../ethereum-json-rpc-api/eth_getTransactionReceipt.md#behaviour-at-pending-on-this-node).

Run the next two examples at once: they read what these two transactions left, which differs from
`latest` only until the block is sealed, within about one slot (12 s).

---

Next: [Read the state that pre-confirmed transactions left](./pending-state.md).
