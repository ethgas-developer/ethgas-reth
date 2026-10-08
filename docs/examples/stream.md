# Stream flashblocks

Example 7 of 7 in [Examples](./README.md). The commands use `WS` of
[Before you start](./README.md#before-you-start).

Subscriptions need a WebSocket or IPC connection. `sub` subscribes, then prints the subscription id
and the next two messages:

```sh
sub() {  # sub PARAMS: the subscription id, then two messages
  echo "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"eth_subscribe\",\"params\":$1}" |
    websocat -n -B 16777216 --max-messages-rev 3 "$WS" | jq -c
}
```

`-B 16777216` raises websocat's message limit from 64 KiB to 16 MiB: a `newFlashblocks` message
carries the whole block, 249 KB for 303 transactions on the test network.

**[`newFlashblocks`](../flashblocks-api/newFlashblocks.md)**, the pending block at each flashblock:

```sh
sub '["newFlashblocks"]'
```

```json
{"jsonrpc":"2.0","id":1,"result":"0xa3c5541bf8f1558c1114ac15b5e4db8d"}
{"jsonrpc":"2.0","method":"eth_subscription","params":{"subscription":"0xa3c5541bf8f1558c1114ac15b5e4db8d","result":{"number":"0x5e","hash":"0x0000000000000000000000000000000000000000000000000000000000000000","gasUsed":"0x696e3f","transactions":[{"hash":"0xfecaea531a3a5b22537322f30349e934ed88e1634b05bc50597338c20c03b3bb","blockHash":null,"blockNumber":"0x5e","transactionIndex":"0x0"}]}}}
{"jsonrpc":"2.0","method":"eth_subscription","params":{"subscription":"0xa3c5541bf8f1558c1114ac15b5e4db8d","result":{"number":"0x5e","hash":"0x0000000000000000000000000000000000000000000000000000000000000000","gasUsed":"0x7c288a","transactions":[{"hash":"0xfecaea531a3a5b22537322f30349e934ed88e1634b05bc50597338c20c03b3bb","blockHash":null,"blockNumber":"0x5e","transactionIndex":"0x0"}]}}}
```

Each message is the whole pending block with full transactions, not a delta: block `0x5e` held 151
transactions in the first message and 178 in the next. Each transaction's `blockHash` is `null`.

**[`pendingLogs`](../flashblocks-api/pendingLogs.md)**, with a filter on the ERC-20 `Transfer` topic:

```sh
sub '["pendingLogs", {"topics": ["0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef"]}]'
```

```json
{"jsonrpc":"2.0","id":1,"result":"0x8b95ab24e440841b308245cea3cde1a9"}
{"jsonrpc":"2.0","method":"eth_subscription","params":{"subscription":"0x8b95ab24e440841b308245cea3cde1a9","result":{"address":"0x2ae2ae036eb3600477cbfce79c1373405a968156","topics":["0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef","0x00000000000000000000000036b8fbdd5e8cf463c05091f5a9992d4d74625b4b","0x000000000000000000000000ade2767fb15d94509fefe8340235e3361d65bab4"],"data":"0x000000000000000000000000000000000000000000000034325a326af5a26724","blockHash":"0x0000000000000000000000000000000000000000000000000000000000000000","blockNumber":"0x5e","transactionHash":"0xea3e2ad168773a9f208df91a86a4920edae31b8fef28f9746bd3a20b1d4720c8","transactionIndex":"0xb4","logIndex":"0x117","removed":false}}}
{"jsonrpc":"2.0","method":"eth_subscription","params":{"subscription":"0x8b95ab24e440841b308245cea3cde1a9","result":{"address":"0xc92a59beae7b9a7599ec02bc0d0a4ffd8c881b03","topics":["0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef","0x000000000000000000000000ade2767fb15d94509fefe8340235e3361d65bab4","0x00000000000000000000000036b8fbdd5e8cf463c05091f5a9992d4d74625b4b"],"data":"0x0000000000000000000000000000000000000000000000000163b3a2352baf8c","blockHash":"0x0000000000000000000000000000000000000000000000000000000000000000","blockNumber":"0x5e","transactionHash":"0xea3e2ad168773a9f208df91a86a4920edae31b8fef28f9746bd3a20b1d4720c8","transactionIndex":"0xb4","logIndex":"0x118","removed":false}}}
```

One log per message: here the two `Transfer` logs of one Uniswap swap. Every pre-confirmed log
has `blockHash` `0x000…0`, so group logs by `transactionHash`.

**[`newFlashblockTransactions`](../flashblocks-api/newFlashblockTransactions.md)**, as hashes, then
as full transactions:

```sh
sub '["newFlashblockTransactions"]'
sub '["newFlashblockTransactions", true]'
```

```json
{"jsonrpc":"2.0","id":1,"result":"0x1395a9fc40de368a3e65ee899bc61cc5"}
{"jsonrpc":"2.0","method":"eth_subscription","params":{"subscription":"0x1395a9fc40de368a3e65ee899bc61cc5","result":"0xf79c551427021829b6c8f4cd0a459546137d85c301bb548f55827cae9b1e6cdc"}}
{"jsonrpc":"2.0","method":"eth_subscription","params":{"subscription":"0x1395a9fc40de368a3e65ee899bc61cc5","result":"0xe2d1f1c967979edec4d92e791b1411e6a90040170b54733522a1606266f3d624"}}
{"jsonrpc":"2.0","id":1,"result":"0x53af34d0b9d108d5d1ccf6c58663d8a9"}
{"jsonrpc":"2.0","method":"eth_subscription","params":{"subscription":"0x53af34d0b9d108d5d1ccf6c58663d8a9","result":{"hash":"0xf69093f0cf57b4b5fb5d9cacd8bd77e9bcb38346d388986581f1427286b049a6","blockHash":null,"blockNumber":"0x5e","transactionIndex":"0xe4","from":"0xe16f66690a41ff50c18cfccab500185d58f53a3b","to":"0x92473ede31d1c7c053df4449466716d431a96dab","value":"0x4a817c800","logs":[],"gasUsed":"0x5208","status":"0x1","cumulativeGasUsed":"0x9f4df1","contractAddress":null}}}
```

The full form adds the receipt fields that flashblock execution knows: `logs`, `gasUsed`,
`status`, `cumulativeGasUsed`, `contractAddress` and `logsBloom`. `blockHash` stays `null` until
the block is sealed.

**Limits.** One connection holds at most 4 `newFlashblocks` subscriptions and 32 of the other two
kinds. A subscription whose client stops reading ends after 10 s, and a subscriber that falls 20
flashblocks behind skips them. See
[`eth_subscribe`](../ethereum-json-rpc-api/eth_subscribe.md#limits).

---

Next: the same flows in a client library: [ethers](./ethers.md), [viem](./viem.md),
[web3.py](./web3py.md) or [alloy](./alloy.md).
