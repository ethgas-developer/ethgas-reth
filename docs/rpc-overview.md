# ETHGas RPC Overview

Which JSON-RPC methods an `ethgas-node` serves, and what each one does with the `pending` tag.

`ethgas-node` is a [reth](https://github.com/paradigmxyz/reth)-based Ethereum execution node with
flashblocks support. It serves the standard Ethereum JSON-RPC interface. When you connect it to a
flashblocks producer, the `pending` block tag stops meaning "a block the node invented from its own
mempool" and starts meaning "the pre-confirmed block the builder is currently assembling".

> **Proofs are the one exception.** `eth_getProof`, `eth_getMultiProof` and `eth_getAccount` refuse
> the `pending` tag while it is the flashblock being built, because pre-confirmed state has no
> state root to prove against. See [`eth_getProof`](./ethereum-json-rpc-api/eth_getProof.md).

---

## Contents

- [What `pending` means on this node](#what-pending-means-on-this-node)
- [After the Amsterdam fork](#after-the-amsterdam-fork)
- [API reference](#api-reference)
  - [Ethereum JSON-RPC API](#ethereum-json-rpc-api)
  - [ETHGas namespace](#ethgas-namespace)
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
| **The next block, executed but not yet confirmed** | For those methods whenever no flashblock data is available and the node has already executed the next block for its consensus client |
| **The latest confirmed block** | For everything else that takes a block tag, and for the methods marked **Yes** whenever neither of the above exists |
| **An error** | For the three proof methods, which cannot prove pre-confirmed state |

Two guarantees follow, and both are deliberate:

1. **`pending` is never invented from a mempool.** If no flashblock data is available, you get a
   real block that was actually executed — the next block when the node already holds it, else
   the latest confirmed block — never a speculative one assembled locally.
2. **A sealed block always wins.** If the network has already produced the block, you are served
   that block rather than a reconstruction of it.

The second guarantee has one visible effect. When the node has executed the next block for its
consensus client but has not yet made it canonical, and that block is at least as new as the
flashblock, the methods that read account state or execute calls answer `pending` from that
executed block instead of from the flashblock. The three proof methods answer from it too, instead
of refusing. `eth_getBlockByNumber`, `eth_getBlockTransactionCountByNumber`,
`eth_getBlockReceipts`, `eth_getTransactionByBlockNumberAndIndex` and `eth_getLogs` keep describing
the flashblock. Without flashblock data, they describe that executed block too, so every method
tells one story. `eth_feeHistory` always uses the latest confirmed block.

Code and storage at `pending`, and balances the builder did not report, come from the node's own
execution of the flashblock transactions on top of the latest confirmed block. After the Amsterdam
fork, that execution runs in the slot the builder sent, so `SLOTNUM` reads what it read for the
builder. `eth_call`,
`eth_estimateGas`, `eth_simulateV1` and the call-tracing methods run on that same state, in the
block environment of the flashblock being built, so they all agree.

One window remains. A call at `pending` reads the pre-confirmed state twice, once for the block
environment and once for the account state. When the pending block changes between the two reads,
because the builder started the next block or the node executed or confirmed one, the call runs
the earlier block environment over the later state. The window is one call wide and opens only at
a block boundary. Repeat the call if the result must be consistent.

---

## After the Amsterdam fork

On a chain where Amsterdam is active, the pending block follows the fork in these ways:

| What | At `pending` |
|---|---|
| `slotNumber` in the block header | The slot the builder is building for |
| `blockAccessListHash` in the block header | Absent: the node builds no access list for an unsealed block |
| `SLOTNUM` in pre-confirmed execution, `eth_call`, `eth_estimateGas` and call tracing | The builder's slot |
| `SLOTNUM` in `eth_simulateV1` | The latest confirmed block's slot plus one per simulated block |
| `eth_getBlockAccessList` | `null` |

See [`eth_getBlockByNumber`](./ethereum-json-rpc-api/eth_getBlockByNumber.md) and
[`eth_simulateV1`](./flashblocks-api/eth_simulateV1.md).

---

## API reference

> **Info.** **Yes** means `pending` reflects the flashblock being built. **No** means `pending`
> returns the latest confirmed block. **Refused** means the call returns an error for `pending`
> while `pending` is the flashblock being built.
> **n/a** means the method takes no block parameter.

### Ethereum JSON-RPC API

Methods where this node does something you need to know about have their own page under
`ethereum-json-rpc-api/`, with parameters, return shape, a worked example and their exact
`pending` behaviour. The rest behave as they do on any Ethereum node. With `pending`, an unlisted
method that takes a block tag answers from the next block when the node has already executed it
for its consensus client, else from the latest confirmed block; none of them reads the flashblock.
Two exceptions: `eth_getBlockAccessList` returns `null` for `pending`, and `debug_getRawHeader`,
`debug_getRawBlock`, `debug_getRawReceipts` and `debug_getRawTransactions` answer from that
executed block or fail (`debug_getRawTransactions` returns an empty list), never from the latest
confirmed block.

| Method | Description | Flashblocks `pending` |
|---|---|:---:|
| [`eth_call`](./ethereum-json-rpc-api/eth_call.md) | Execute a call without creating a transaction | **Yes** |
| [`eth_estimateGas`](./ethereum-json-rpc-api/eth_estimateGas.md) | Estimate gas for a transaction | **Yes** |
| [`eth_simulateV1`](./flashblocks-api/eth_simulateV1.md) | Simulate transaction bundles against pre-confirmed state | **Yes** |
| [`eth_getBalance`](./ethereum-json-rpc-api/eth_getBalance.md) | Account balance | **Yes** |
| [`eth_getTransactionCount`](./ethereum-json-rpc-api/eth_getTransactionCount.md) | Account nonce | **Yes** |
| [`eth_getCode`](./ethereum-json-rpc-api/eth_getCode.md) | Contract bytecode | **Yes** |
| [`eth_getStorageAt`](./ethereum-json-rpc-api/eth_getStorageAt.md) | Contract storage slot | **Yes** |
| `eth_getAccountInfo` | Account balance, nonce and code in one call | **Yes** |
| `eth_getStorageValues` | Several storage slots in one call | **Yes** |
| [`eth_getProof`](./ethereum-json-rpc-api/eth_getProof.md) | Merkle proof of an account and its storage | **Refused** |
| `eth_getMultiProof` | Merkle proofs of several accounts in one call | **Refused** |
| `eth_getAccount` | Account with its storage root | **Refused** |
| `eth_createAccessList` | Access list a transaction would need | **Yes** |
| `debug_traceCall` | Trace a call | **Yes** |
| `trace_call`, `trace_callMany`, `trace_rawTransaction` | Trace calls, in the `trace` namespace | **Yes** |
| [`eth_getBlockByNumber`](./ethereum-json-rpc-api/eth_getBlockByNumber.md) | Block by number or tag | **Yes** |
| [`eth_getBlockTransactionCountByNumber`](./ethereum-json-rpc-api/eth_getBlockTransactionCountByNumber.md) | Transaction count of a block | **Yes** |
| [`eth_getBlockReceipts`](./ethereum-json-rpc-api/eth_getBlockReceipts.md) | All receipts in a block | **Yes** |
| [`eth_getTransactionByBlockNumberAndIndex`](./ethereum-json-rpc-api/eth_getTransactionByBlockNumberAndIndex.md) | Transaction by block and index | **Yes** |
| [`eth_getLogs`](./ethereum-json-rpc-api/eth_getLogs.md) | Logs matching a filter, including pending flashblock logs | **Yes** |
| [`eth_getTransactionByHash`](./ethereum-json-rpc-api/eth_getTransactionByHash.md) | Transaction by hash. Pre-confirmed included, unless this node's mempool holds it | **Yes** |
| [`eth_getTransactionReceipt`](./ethereum-json-rpc-api/eth_getTransactionReceipt.md) | Receipt by hash, pre-confirmed included | **Yes** |
| [`eth_sendRawTransaction`](./ethereum-json-rpc-api/eth_sendRawTransaction.md) | Submit a signed transaction | n/a |
| [`eth_sendRawTransactionSync`](./flashblocks-api/eth_sendRawTransactionSync.md) | Submit and wait for flashblock inclusion | n/a |
| [`eth_getBlockByHash`](./ethereum-json-rpc-api/eth_getBlockByHash.md) | Block by hash | n/a |
| `eth_getBlockTransactionCountByHash` | Transaction count by block hash | n/a |
| `eth_getTransactionByBlockHashAndIndex` | Transaction by block hash and index | n/a |
| `eth_blockNumber` | Latest confirmed block number | n/a |
| `eth_gasPrice` | Current gas price. With flashblocks, the next block's base fee plus the builder's inclusion fee; see [`ethgas_inclusionPriorityFee`](./flashblocks-api/ethgas_inclusionPriorityFee.md) | n/a |
| `eth_maxPriorityFeePerGas` | Suggested priority fee. With flashblocks, the builder's inclusion fee; see [`ethgas_inclusionPriorityFee`](./flashblocks-api/ethgas_inclusionPriorityFee.md) | n/a |
| `eth_feeHistory` | Historical fee data | **No** |
| `eth_chainId` | Chain ID | n/a |
| `eth_syncing` | Sync status | n/a |
| `net_version` | Network ID | n/a |
| `web3_clientVersion` | Client version. Carries an `ethgas/v<version>` segment | n/a |
| [`eth_subscribe`](./ethereum-json-rpc-api/eth_subscribe.md) / `eth_unsubscribe` | Manage subscriptions | see below |

#### `eth_sendRawTransactionSync`

Submits a transaction and waits for it to appear in a flashblock, then returns its receipt. This is
the method to use when you want pre-confirmation rather than a transaction hash.

The `timeout_ms` parameter is optional. The node clamps it to its configured maximum,
`--rpc.send-raw-transaction-sync-timeout`, 30 s unless the operator sets it. `0` or absent waits
that maximum.

```json
{
  "jsonrpc": "2.0",
  "method": "eth_sendRawTransactionSync",
  "params": ["0x02f8...", 3000],
  "id": 1
}
```

### ETHGas namespace

Available only with `--flashblocks-url`, on every configured transport, whatever `--http.api` says.

| Method | Description | Flashblocks `pending` |
|---|---|:---:|
| [`ethgas_inclusionPriorityFee`](./flashblocks-api/ethgas_inclusionPriorityFee.md) | The priority fee that clears the builder's inclusion gate, with its source and age | n/a |

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
