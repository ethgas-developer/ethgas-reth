# ETHGas RPC Overview

Which JSON-RPC methods an `ethgas-node` serves, and what each one does with the `pending` tag.

`ethgas-node` is a [reth](https://github.com/paradigmxyz/reth)-based Ethereum execution node with
flashblocks support. It serves the standard Ethereum JSON-RPC interface. When you connect it to a
flashblocks producer, the `pending` block tag stops meaning "a block the node invented from its own
mempool" and starts meaning "the pre-confirmed block the builder is currently assembling".

> **Some pre-confirmed state reads are not available yet.** Methods marked **Not yet** below
> currently answer `pending` with the latest confirmed block.

---

## Contents

- [What `pending` means on this node](#what-pending-means-on-this-node)
- [API reference](#api-reference)
  - [Ethereum JSON-RPC API](#ethereum-json-rpc-api)
  - [Flashblocks subscriptions](#flashblocks-subscriptions)

---

## What `pending` means on this node

This is the part that differs most from other nodes.

A typical node answers `pending` with a block it assembles from its own mempool: transactions no
builder selected, in an order no builder chose. This node never does that. Here, `pending` means the
block the builder is actually assembling right now.

| `pending` resolves to | When |
|---|---|
| **The flashblock being built** | For the methods marked **Yes** below |
| **The latest confirmed block** | For everything else that takes a block tag, and whenever no flashblock data is available |

Two guarantees follow, and both are deliberate:

1. **`pending` is never invented from a mempool.** If no flashblock data is available, you get a
   real block that was actually executed — never a speculative one assembled locally.
2. **A sealed block always wins.** If the network has already produced the block, you are served
   that block rather than a reconstruction of it.

---

## API reference

> **Info.** **Yes** means `pending` reflects the flashblock being built. **Not yet** means that is
> planned, and `pending` currently returns the latest confirmed block. **No** means `pending`
> returns the latest confirmed block. **n/a** means the method takes no block parameter.

### Ethereum JSON-RPC API

Methods where this node does something you need to know about have their own page under
`ethereum-json-rpc-api/`, with parameters, return shape, a worked example and their exact
`pending` behaviour. The rest behave as they do on any Ethereum node.

| Method | Description | Flashblocks `pending` |
|---|---|:---:|
| [`eth_call`](./ethereum-json-rpc-api/eth_call.md) | Execute a call without creating a transaction | **Yes** |
| [`eth_estimateGas`](./ethereum-json-rpc-api/eth_estimateGas.md) | Estimate gas for a transaction | **Yes** |
| [`eth_simulateV1`](./flashblocks-api/eth_simulateV1.md) | Simulate transaction bundles against pre-confirmed state | **Yes** |
| [`eth_getBalance`](./ethereum-json-rpc-api/eth_getBalance.md) | Account balance | **Yes**, for reported balances |
| [`eth_getTransactionCount`](./ethereum-json-rpc-api/eth_getTransactionCount.md) | Account nonce | **Yes** |
| [`eth_getCode`](./ethereum-json-rpc-api/eth_getCode.md) | Contract bytecode | **Not yet** |
| [`eth_getStorageAt`](./ethereum-json-rpc-api/eth_getStorageAt.md) | Contract storage slot | **Not yet** |
| `eth_getAccountInfo` | Account balance, nonce and code in one call | **Not yet** |
| `eth_getStorageValues` | Several storage slots in one call | **Not yet** |
| [`eth_getBlockByNumber`](./ethereum-json-rpc-api/eth_getBlockByNumber.md) | Block by number or tag | **Yes** |
| [`eth_getBlockTransactionCountByNumber`](./ethereum-json-rpc-api/eth_getBlockTransactionCountByNumber.md) | Transaction count of a block | **Yes** |
| [`eth_getLogs`](./ethereum-json-rpc-api/eth_getLogs.md) | Logs matching a filter, including pending flashblock logs | **Yes** |
| [`eth_getTransactionByHash`](./ethereum-json-rpc-api/eth_getTransactionByHash.md) | Transaction by hash, pre-confirmed included | **Yes** |
| [`eth_getTransactionReceipt`](./ethereum-json-rpc-api/eth_getTransactionReceipt.md) | Receipt by hash, pre-confirmed included | **Yes** |
| [`eth_sendRawTransaction`](./ethereum-json-rpc-api/eth_sendRawTransaction.md) | Submit a signed transaction | n/a |
| [`eth_sendRawTransactionSync`](./flashblocks-api/eth_sendRawTransactionSync.md) | Submit and wait for flashblock inclusion | n/a |
| [`eth_getBlockReceipts`](./ethereum-json-rpc-api/eth_getBlockReceipts.md) | All receipts in a block | **No** |
| [`eth_getTransactionByBlockNumberAndIndex`](./ethereum-json-rpc-api/eth_getTransactionByBlockNumberAndIndex.md) | Transaction by block and index | **No** |
| [`eth_getBlockByHash`](./ethereum-json-rpc-api/eth_getBlockByHash.md) | Block by hash | n/a |
| `eth_getBlockTransactionCountByHash` | Transaction count by block hash | n/a |
| `eth_getTransactionByBlockHashAndIndex` | Transaction by block hash and index | n/a |
| `eth_blockNumber` | Latest confirmed block number | n/a |
| `eth_gasPrice` | Current gas price | n/a |
| `eth_maxPriorityFeePerGas` | Suggested priority fee | n/a |
| `eth_feeHistory` | Historical fee data | **No** |
| `eth_chainId` | Chain ID | n/a |
| `eth_syncing` | Sync status | n/a |
| `net_version` | Network ID | n/a |
| `web3_clientVersion` | Client version. Carries an `ethgas/v<version>` segment | n/a |
| [`eth_subscribe`](./ethereum-json-rpc-api/eth_subscribe.md) / `eth_unsubscribe` | Manage subscriptions | see below |

#### `eth_sendRawTransactionSync`

Submits a transaction and waits for it to appear in a flashblock, then returns its receipt. This is
the method to use when you want pre-confirmation rather than a transaction hash.

The `timeout_ms` parameter is optional. It defaults to **6000 ms**, which is also the maximum. A
larger value is rejected with `-32602`.

```json
{
  "jsonrpc": "2.0",
  "method": "eth_sendRawTransactionSync",
  "params": ["0x02f8...", 3000],
  "id": 1
}
```

### Flashblocks subscriptions

Available through `eth_subscribe` when the node is connected to a flashblocks feed. The standard
subscriptions (`newHeads`, `logs`, `newPendingTransactions`, `syncing`) continue to work unchanged.

| Subscription | Parameter | Fires with |
|---|---|---|
| [`newFlashblocks`](./flashblocks-api/newFlashblocks.md) | none | The current pending block, on every new flashblock |
| [`pendingLogs`](./flashblocks-api/pendingLogs.md) | a log filter | Logs from the latest flashblock that match the filter |
| [`newFlashblockTransactions`](./flashblocks-api/newFlashblockTransactions.md) | see below | Transactions from the latest flashblock |

`newFlashblockTransactions` accepts three parameter forms:

- `true` — full transaction, with its logs and gas used
- a log filter — full transactions where any log matches
- no parameter — transaction hashes only

```json
{
  "jsonrpc": "2.0",
  "method": "eth_subscribe",
  "params": ["newFlashblockTransactions", true],
  "id": 1
}
```
