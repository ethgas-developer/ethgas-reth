# Wait for the seal

Example 6 of 7 in [Examples](./README.md). The commands use `rpc` and the variables of
[Before you start](./README.md#before-you-start). `HASH` is the second transaction of
[Send a transaction and wait for its pre-confirmation](./send-and-wait.md).

Poll the receipt until its `blockHash` is non-zero:

```sh
until rpc eth_getTransactionReceipt "[\"$HASH\"]" | jq -e '.result.blockHash | test("[1-9a-f]")' > /dev/null; do sleep 1; done
rpc eth_getTransactionReceipt "[\"$HASH\"]"
```

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "status": "0x1",
    "transactionHash": "0xa4c188efeb01465e10ebd177a25a5df125f4532ffcf5bff8687b8984d24a73cb",
    "transactionIndex": "0xe4",
    "blockHash": "0x102efadf7acc18b4c5691affe6160e75618d890464f10fce7e8739d16d85f4ca",
    "blockNumber": "0x5d",
    "gasUsed": "0x670e"
  }
}
```

The poll waited 8 s. The same builder sealed block `0x5d`, so the receipt kept its block number
and index, and gained the block's hash. A receipt with a non-zero `blockHash` is in a sealed block.

---

Next: [Stream flashblocks](./stream.md).
