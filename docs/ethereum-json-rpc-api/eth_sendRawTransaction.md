# eth_sendRawTransaction

Submits a signed transaction to the node.

| | |
|---|---|
| At `pending` | No block parameter |

## Parameters

| Name | Type | Required | Description |
|---|---|---|---|
| `data` | string | yes | The signed transaction, RLP-encoded as a hex string |

## Returns

`string`: the 32-byte transaction hash, if the transaction was accepted.

## Example

**Request**

```json
{
  "jsonrpc": "2.0",
  "method": "eth_sendRawTransaction",
  "params": ["0x02f8…"],
  "id": 1
}
```

**Response**

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": "0x7f4e2a8c1b6d9035e4a7c2f8b1d6e9a34c7f0b5d8e2a6c9f3b7d1e4a8c2f6b0c"
}
```

## Behaviour at `pending` on this node

The method returns as soon as the transaction is accepted, and the hash it returns says nothing
about inclusion. To submit and wait for pre-confirmation in one call, use
[`eth_sendRawTransactionSync`](../flashblocks-api/eth_sendRawTransactionSync.md).

**The mempool checks a transaction against the latest block, not `pending`.** So it accepts a
replacement (the same nonce, higher fees) for a transaction that is already pre-confirmed. Only one
of the two can land: when the pre-confirmed transaction is sealed, the replacement never lands.

A transaction with a nonce gap, or with a maximum fee below the next base fee, is accepted without
an error and waits in the mempool.

## Errors

| Code | Message | When |
|---|---|---|
| `-32000` | `nonce too low` | The nonce is below the account's nonce at the latest block |
| `-32003` | `insufficient funds for gas * price + value` | The balance at the latest block does not cover the transaction |
| `-32000` | `already known` | This node's mempool holds the transaction, also while it is pre-confirmed |
| `-32000` | `replacement transaction underpriced` | The mempool holds a transaction with the same nonce, and the new one does not raise both fees by at least 10 % (100 % for a blob transaction) |
| `-32003` | `txpool is full` | The mempool has no room |
| `-32000` | `tx fee (… wei) exceeds the configured cap (… wei)` | Gas limit times maximum fee is above `--rpc.txfeecap`, 1 ETH by default |
| `-32000` | `in-flight transaction limit reached for delegated accounts` | The sender has an EIP-7702 delegation and already has a transaction in the mempool |

The message can carry details after a colon. These are the common refusals; the node returns
reth's other transaction pool errors too.
