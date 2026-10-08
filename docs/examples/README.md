# Examples

Requests and answers for the methods that serve flashblock data, one example a page. In the order
below they follow an application's session: read the pre-confirmed block, price a transaction, send
it, read the state it left, simulate the next step, wait for the seal, and stream what the builder
sequences. The library pages do the same in ethers and viem (JavaScript), web3.py (Python) and
alloy (Rust).

The answers come from a test network (chain id 3151908) whose builder publishes a flashblock every
second and seals a block every 12 s. They are shortened: fields and list items are left out, and
`…` marks a cut inside a value. Each method's own page has its full behaviour; follow the links.

---

## With curl and websocat

Run them in order, in one terminal. Examples 4 to 6 use the variables and the transactions of
example 3, and examples 4 and 5 must run before its block is sealed.

| | Example | Methods |
|---|---|---|
| 1 | [Read the pre-confirmed block](./pending-block.md) | `eth_getBlockByNumber` |
| 2 | [Price a transaction for the next flashblock](./price.md) | `ethgas_inclusionPriorityFee`, `eth_maxPriorityFeePerGas`, `eth_fillTransaction` |
| 3 | [Send a transaction and wait for its pre-confirmation](./send-and-wait.md) | `eth_sendRawTransactionSync`, `eth_sendRawTransaction`, `eth_getTransactionReceipt` |
| 4 | [Read the state that pre-confirmed transactions left](./pending-state.md) | `eth_getBalance`, `eth_getTransactionCount`, `eth_getAccountInfo`, `eth_call` |
| 5 | [Simulate at `pending`](./simulate.md) | `eth_estimateGas`, `eth_simulateV1` |
| 6 | [Wait for the seal](./wait-for-the-seal.md) | `eth_getTransactionReceipt` |
| 7 | [Stream flashblocks](./stream.md) | `newFlashblocks`, `pendingLogs`, `newFlashblockTransactions` |

## In client libraries

Each program reads at `pending`, sends 1 wei and waits for its pre-confirmation, then prints three
stream messages.

| Library | Language | Sends with | Streams |
|---|---|---|---|
| [ethers 6](./ethers.md) | JavaScript | `eth_sendRawTransactionSync`, through `provider.send` | `newFlashblocks` |
| [viem 2](./viem.md) | JavaScript | `sendTransactionSync` | `newFlashblocks` |
| [web3.py 7](./web3py.md) | Python | `eth_sendRawTransactionSync`, through `make_request` | `newFlashblockTransactions`, hashes |
| [alloy 2](./alloy.md) | Rust | `send_transaction_sync` | `newFlashblocks` |

What these libraries do with this node's answers elsewhere is on
[Client libraries](../client-libraries.md).

---

## Before you start

The examples use `curl` and `jq` over HTTP, `websocat` over WebSocket, and Foundry's `cast` to sign
transactions. Point them at your node and an account:

```sh
export RPC=http://localhost:8545
export WS=ws://localhost:8546
export PRIVATE_KEY=0x…  # the key of the account the examples send from
export FROM=0x…         # its address
export TO=0x…           # a new address: example 3 sends it 1 ETH
export COUNTER=0x…      # a contract with number() and increment(), such as Foundry's sample Counter
```

`rpc` sends one JSON-RPC request:

```sh
rpc() {  # rpc METHOD [PARAMS]
  curl -s "$RPC" -H 'Content-Type: application/json' \
    -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"$1\",\"params\":${2:-[]}}" | jq
}
```
