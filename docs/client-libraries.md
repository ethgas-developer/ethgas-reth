# Client libraries

What common client libraries do with this node's answers, and what to do about it. The behaviour
below comes from each library's source: ethers 6.17.0, viem 2.57.3, web3.py 7.16.0, alloy 2.5.0
and the MetaMask extension 13.50.0.

## Nonces

`eth_getTransactionCount(address, "pending")` counts the account's transactions in flashblocks and
its transactions in this node's mempool. Libraries that ask `pending` before every send (web3.py,
viem when the node does not fill the transaction) therefore get a new nonce for each of two quick
sends through this node. viem's default path calls `eth_fillTransaction`, whose nonce is that same
`pending` count. MetaMask keeps its own nonces.

An ethers `Wallet` asks `pending` before every send too, but its provider answers a request that
repeats one from the last 250 ms from its own cache (`cacheTimeout`), without asking the node. So a
second send within 250 ms of the first gets the same nonce, and the node refuses it: `already known`
when the two transactions are identical (ethers: "could not coalesce error"), else `replacement
transaction underpriced`. The first transaction stands. For quick sends, use ethers'
`NonceManager`, or create the provider with `cacheTimeout: -1`.

A transaction sent through another node is not in this node's mempool, so it is counted only once
it is in a flashblock. If you send through several nodes, track nonces locally: ethers
`NonceManager`, viem `nonceManager`, or alloy's default `CachedNonceManager`.

## Waiting for a transaction

A receipt with `blockHash: 0x000…0` is pre-confirmed, not sealed, and it can still change or
vanish; see [`eth_getTransactionReceipt`](./ethereum-json-rpc-api/eth_getTransactionReceipt.md).

| Library | Call | What it returns on this node |
|---|---|---|
| ethers | `tx.wait()` | the pre-confirmed receipt when its first fetch finds one (`confirmations()` is 0), else the sealed receipt. `provider.waitForTransaction(hash)` waits for the seal |
| viem | `waitForTransactionReceipt` | the pre-confirmed receipt when its first fetch finds one, else the sealed receipt. **With `confirmations` of 2 or more it keeps the first receipt it fetched and returns that one after the seal** |
| web3.py | `wait_for_transaction_receipt` | the pre-confirmed receipt, about 1 s after the send |
| alloy | `get_receipt()` | any receipt a poll finds, the pre-confirmed one included; `watch()` waits for the seal |
| MetaMask | — | never confirms before the seal. A pre-confirmed success leaves the warning "There was a problem loading this transaction." on the transaction; a pre-confirmed failure is final at once |

To wait for the seal, poll `eth_getTransactionReceipt` until `blockHash` is non-zero.

## Gas estimates

Without a block parameter, `eth_estimateGas` runs at `pending` while flashblock data is served, so
an estimate sees the pre-confirmed transactions before it, such as an approval. ethers, MetaMask
and viem send no block parameter, and viem's `eth_fillTransaction` path estimates at `pending`.
web3.py sends `"latest"`: pass `block_identifier="pending"` for the same effect. alloy estimates
at `pending` by default.

## Fees

The builder's inclusion fee reaches `eth_maxPriorityFeePerGas`, `eth_gasPrice` and
`eth_fillTransaction`; see [`ethgas_inclusionPriorityFee`](./flashblocks-api/ethgas_inclusionPriorityFee.md).
ethers, web3.py and viem pay it. alloy's default fee estimator reads `eth_feeHistory` and MetaMask
uses its own gas service, so they pay a historical tip, and their transactions wait for the
block's last flashblock.

## Errors

- The mempool's refusals have code `-32000` or `-32003` and the reason in the message; see
  [`eth_sendRawTransaction`](./ethereum-json-rpc-api/eth_sendRawTransaction.md). ethers reports
  `already known` and several other reasons as `UNKNOWN_ERROR` "could not coalesce error", and every
  `eth_sendRawTransactionSync` error the same way: read the original message in `error.error`.
  viem reports `already known` as `NonceTooLowError`.
- A revert has code `3`, with the revert data in `data` when the contract returns some. Without
  revert data, ethers says "missing revert data", viem "reverted for an unknown reason", and
  web3.py raises `ContractLogicError` with `data` set to `'no data'`.
- The `eth_sendRawTransactionSync` timeout has code `4`, and the transaction hash appears only in
  the message. The transaction can still land: look it up by its hash.

## Subscriptions

- No library has a typed API for `newFlashblocks`, `newFlashblockTransactions` or `pendingLogs`:
  send `eth_subscribe` yourself.
- web3.py 7 cannot format the full form of `newFlashblockTransactions` and raises `TypeError`. Use
  the hashes form, or read the raw messages.
- viem's `watchBlocks({ blockTag: "pending" })` emits once per block number, so it shows only the
  first flashblock of each block. Subscribe to `newFlashblocks` instead.
- ethers' `Block.isMined()` is `true` for the pending block, because its hash `0x000…0` is not
  null, and `getBlock()` on a pre-confirmed receipt or log fails.
- The node limits flashblocks subscriptions per connection and ends those whose client stops
  reading; see [`eth_subscribe`](./ethereum-json-rpc-api/eth_subscribe.md#limits).
