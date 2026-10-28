# RPC overview

`ethgas-node` is an Ethereum execution client built on [reth](https://github.com/paradigmxyz/reth).
It serves the standard Ethereum JSON-RPC API.

With `--flashblocks-url`, the node also reads the builder's flashblocks feed. A flashblock is a part
of a block that the builder streams before it seals the block. The node executes each flashblock and
serves the result as the **pre-confirmed block**: the block the builder is building, up to its
latest flashblock.

This page lists which methods read the pre-confirmed block, and what each method returns for the
`pending` block tag.

## `pending` on this node

By default, reth builds `pending` from its mempool. This node never does.

- For the methods marked **Pre-confirmed** below, `pending` is the pre-confirmed block when the
  node has one.
- Otherwise, `pending` is the latest block. While the node imports a block (it has executed the
  block, and its consensus client has not yet made it canonical), `pending` is that block.

The node has a pre-confirmed block while it holds usable flashblocks for a block after the latest
block. It has none when it runs without `--flashblocks-url`, while it syncs, before the builder
sends the first flashblock of the next block, and after it
[rejects a flashblock](#rejected-flashblocks).

## Methods

- **Pre-confirmed**: at `pending`, the method reads the pre-confirmed block when the node has one.
  [At a block boundary](#at-a-block-boundary) lists the exceptions.
- **Latest**: at `pending`, the method reads the latest block or the block being imported. It never
  reads the pre-confirmed block.

Methods this page does not list behave as on reth. The node does not serve `eth_getProof`,
`eth_getMultiProof` or `eth_getAccount`. They return `-32601` (method not found).
`web3_clientVersion` adds an `ethgas/v<version>` segment to reth's version string.

### Account state

| Method | At `pending` |
|---|---|
| [`eth_getBalance`](./ethereum-json-rpc-api/eth_getBalance.md) | **Pre-confirmed** |
| [`eth_getTransactionCount`](./ethereum-json-rpc-api/eth_getTransactionCount.md) | **Pre-confirmed**, then raised past the account's transactions in this node's mempool |
| [`eth_getCode`](./ethereum-json-rpc-api/eth_getCode.md) | **Pre-confirmed** |
| [`eth_getStorageAt`](./ethereum-json-rpc-api/eth_getStorageAt.md) | **Pre-confirmed** |
| `eth_getAccountInfo` | **Pre-confirmed** |
| `eth_getStorageValues` | **Pre-confirmed** |

### Calls and simulation

| Method | At `pending` |
|---|---|
| [`eth_call`](./ethereum-json-rpc-api/eth_call.md) | **Pre-confirmed** |
| [`eth_estimateGas`](./ethereum-json-rpc-api/eth_estimateGas.md) | **Pre-confirmed**. Without a block parameter, the estimate also runs at `pending` while the node has a pre-confirmed block |
| `eth_createAccessList` | **Pre-confirmed** |
| [`eth_simulateV1`](./flashblocks-api/eth_simulateV1.md) | **Pre-confirmed** state. The simulated blocks follow the latest block, not the pre-confirmed block |
| `debug_traceCall` | **Pre-confirmed**. **Latest** when `txIndex` is set |
| `trace_call`, `trace_callMany`, `trace_rawTransaction` | **Pre-confirmed** |
| `eth_callMany`, `eth_callBundle`, `debug_traceCallMany` | **Latest** |

### Blocks and logs

| Method | At `pending` |
|---|---|
| [`eth_getBlockByNumber`](./ethereum-json-rpc-api/eth_getBlockByNumber.md) | **Pre-confirmed** |
| [`eth_getBlockTransactionCountByNumber`](./ethereum-json-rpc-api/eth_getBlockTransactionCountByNumber.md) | **Pre-confirmed** |
| [`eth_getBlockReceipts`](./ethereum-json-rpc-api/eth_getBlockReceipts.md) | **Pre-confirmed** |
| [`eth_getTransactionByBlockNumberAndIndex`](./ethereum-json-rpc-api/eth_getTransactionByBlockNumberAndIndex.md) | **Pre-confirmed** |
| [`eth_getLogs`](./ethereum-json-rpc-api/eth_getLogs.md) | **Pre-confirmed** when `toBlock` is `pending` |
| `eth_getHeaderByNumber` | **Latest** |
| `eth_getRawTransactionByBlockNumberAndIndex` | **Latest** |
| `eth_getBlockAccessList`, `eth_getBlockAccessListByBlockNumber` | `null` |
| `eth_getBlockAccessListRaw`, `debug_getRawBlockAccessList` | **Latest** |

The pre-confirmed block's `hash` is `0x000…0`, so
[`eth_getBlockByHash`](./ethereum-json-rpc-api/eth_getBlockByHash.md) cannot fetch it.

### Transactions

| Method | On this node |
|---|---|
| [`eth_getTransactionByHash`](./ethereum-json-rpc-api/eth_getTransactionByHash.md) | Also finds pre-confirmed transactions. A transaction in this node's mempool has no block fields until its block is sealed |
| [`eth_getTransactionReceipt`](./ethereum-json-rpc-api/eth_getTransactionReceipt.md) | Also returns pre-confirmed receipts. Their `blockHash` is `0x000…0`. A pre-confirmed receipt does not prove that the transaction is in a sealed block |
| [`eth_sendRawTransaction`](./ethereum-json-rpc-api/eth_sendRawTransaction.md) | Returns the hash when the node accepts the transaction |
| [`eth_sendRawTransactionSync`](./flashblocks-api/eth_sendRawTransactionSync.md) | Waits until the transaction is pre-confirmed or sealed, then returns its receipt |

### Fees

| Method | On this node |
|---|---|
| `eth_maxPriorityFeePerGas` | The builder's inclusion fee while the node serves it, else reth's suggestion |
| `eth_gasPrice` | The next block's base fee plus the builder's inclusion fee while the node serves that fee, else reth's gas price |
| `eth_fillTransaction` | Fills a missing nonce with the `pending` transaction count. Gives a request without fee fields the builder's inclusion fee as its priority fee. Estimates a missing gas limit at `pending` |
| `eth_feeHistory` | Answers `pending` with the latest block |

[`ethgas_inclusionPriorityFee`](./flashblocks-api/ethgas_inclusionPriorityFee.md) says when the node
serves the builder's inclusion fee.

### `ethgas` namespace

[`ethgas_inclusionPriorityFee`](./flashblocks-api/ethgas_inclusionPriorityFee.md) returns the
priority fee that clears the builder's inclusion gate, with its source and age. The node serves it
with `--flashblocks-url`, on every enabled transport, whatever `--http.api` selects.

### Subscriptions

With `--flashblocks-url`, `eth_subscribe` takes four more subscription types, over WebSocket or
IPC. The standard types (`newHeads`, `logs`, `newPendingTransactions` and `syncing`) work as on
reth.

| Subscription | Parameter | Each message |
|---|---|---|
| [`newFlashblocks`](./flashblocks-api/newFlashblocks.md) | None | The pre-confirmed block, after a new flashblock |
| [`pendingLogs`](./flashblocks-api/pendingLogs.md) | A log filter, optional | One log of the latest flashblock that matches the filter |
| [`newFlashblockTransactions`](./flashblocks-api/newFlashblockTransactions.md) | `true`, a log filter, or none | One transaction of the latest flashblock: in full with its logs for `true`, in full when one of its logs matches for a filter, as a hash for none |
| [`newFlashblockBatches`](./flashblocks-api/newFlashblockBatches.md) | A log filter, optional | The latest flashblock, once: every transaction it added, in full with its logs, or for a filter those with a matching log; also when none match |

### `relay` namespace

With `--enable-simulator`, the node serves a relay's block simulation requests:
`relay_validateBuilderSubmissionV1` to `relay_validateBuilderSubmissionV6`,
`relay_validateMergedBuilderSubmissionV5` and `relay_inclusionList`. It serves them on every enabled
transport, whatever `--http.api` selects.

## How the node builds the pre-confirmed block

The node executes the transactions of each flashblock on top of the latest block, in the block
environment of the block the builder is building. Nonces, code, storage and call results at
`pending` come from this execution.

The receipts come from the builder: status, gas used and logs. The node rejects a flashblock when a
receipt's status differs from its own execution of the transaction.

`eth_getBalance` returns the balance the builder last reported for the address, if the builder
reported one. Every other balance comes from the node's execution, including the balance in
`eth_getAccountInfo`.

The pre-confirmed state does not include two parts of the sealed block:

- the block's withdrawals
- the transactions the builder adds when it seals the block, such as its payment to the fee
  recipient

They show at `pending` after the node imports the sealed block.

## Rejected flashblocks

The node rejects a flashblock that it cannot apply. Examples: a flashblock out of sequence, a
transaction that fails to execute, a receipt status that differs from the node's own execution, and
after the Amsterdam fork a block without a slot number.

The node never serves a rejected flashblock, or a later flashblock of the same block. It can keep
the block's earlier flashblocks until the next flashblock arrives. After that, it has no
pre-confirmed block until the first flashblock of the builder's next block.

## At a block boundary

**The node imports a block at least as new as the pre-confirmed block.** The account-state, call
and simulation methods answer `pending` from the block being imported. The block methods
(`eth_getBlockByNumber`, `eth_getBlockTransactionCountByNumber`, `eth_getBlockReceipts`,
`eth_getTransactionByBlockNumberAndIndex` and `eth_getLogs`) keep the pre-confirmed block. This
lasts until the block is canonical.

**The builder starts block N+1 before the node imports block N.** The pre-confirmed block is then
N+1 on top of the flashblocks of N. Those flashblocks do not include the transactions the builder
adds when it seals N, such as its payment to the fee recipient. Until the node imports N, state and
calls at `pending` do not include those transactions. In this window, `eth_getLogs` from `pending`
to `pending` returns the logs of both blocks.

**`pending` moves to a new block during a call.** A call at `pending` reads the block environment
first and the state after. When `pending` moves to a new block between the two reads, the call runs
in the old block's environment over the new block's state. Repeat the call if the result must be
exact.

## After the Amsterdam fork

| At `pending` | Value |
|---|---|
| `slotNumber` in the block header | The slot the builder is building for |
| `blockAccessListHash` in the block header | Absent. The node builds no access list for a block that is not sealed |
| `SLOTNUM` in the pre-confirmed block, `eth_call`, `eth_estimateGas` and the call-tracing methods | The builder's slot |
| `SLOTNUM` in `eth_simulateV1` | The previous block's slot plus one |
| `eth_getBlockAccessList` | `null` |

The node rejects the flashblocks of a block without a slot number.

## See also

- [Client libraries](./client-libraries.md): what ethers, viem, web3.py, alloy and MetaMask do with
  these answers
- [Examples](./examples/README.md): the flashblocks-aware methods used together
