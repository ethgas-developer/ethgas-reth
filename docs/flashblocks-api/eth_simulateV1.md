# eth_simulateV1

Simulates one or more bundles of calls against pre-confirmed state, without submitting anything.

| | |
|---|---|
| At `pending` | **Pre-confirmed** state; the simulated blocks follow the latest block |
| Transport | HTTP, WebSocket or IPC |

## Parameters

| Name | Type | Required | Description |
|---|---|---|---|
| `payload` | object | yes | The simulation. See below |
| `block` | string | no | Block number in hex, or a tag. Defaults to `"latest"`. Pass `"pending"` to simulate against pre-confirmed state |

`payload` fields:

| Field | Type | Description |
|---|---|---|
| `blockStateCalls` | array | One entry per simulated block. Each holds `calls`, and optionally `stateOverrides` and `blockOverrides` |
| `traceTransfers` | boolean | Emit ETH transfers as logs. Defaults to `false` |
| `validation` | boolean | Enforce nonce, balance and base fee checks. A call without `maxFeePerGas` or `gasPrice` then pays 0, and the request fails with `-38012`. Defaults to `false` |

Each entry in `blockStateCalls`:

| Field | Type | Description |
|---|---|---|
| `calls` | array | Transaction request objects to execute in order |
| `stateOverrides` | object | Per-address overrides of `balance`, `nonce`, `code` or `state` |
| `blockOverrides` | object | Overrides of block fields such as `number` and `time` |

## Returns

An array of simulated blocks. Each carries the usual block fields plus a `calls` array, one entry
per call, with `status`, `gasUsed`, `maxUsedGas` (the gas used before refunds), `returnData`,
`logs`, and `error` where the call failed.

`stateRoot` on a simulated block is `0x000…0` by default: simulation does not commit to a trie. With
`--rpc.compute-state-root-for-eth-simulate`, each simulated block carries the state root after its
calls; at `pending`, that root includes the pre-confirmed state.

## Example

**Request**

```json
{
  "jsonrpc": "2.0",
  "method": "eth_simulateV1",
  "params": [
    {
      "blockStateCalls": [
        {
          "calls": [
            {
              "from": "0xd3cda913deb6f4967b2ef66ae97de114a83bcc01",
              "to": "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48",
              "gas": "0x186a0",
              "maxFeePerGas": "0x3b9aca00",
              "data": "0x70a082310000000000000000000000004200000000000000000000000000000000000006"
            }
          ]
        }
      ],
      "traceTransfers": true,
      "validation": true
    },
    "pending"
  ],
  "id": 1
}
```

**Response**

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": [
    {
      "number": "0x10f2c5",
      "hash": "0x9b2e7c4f1a6d8053e2b7a4c9f1d6e8b30a5c7f2d9e4b1a6c8f3d7e0b2a5c9f41",
      "stateRoot": "0x0000000000000000000000000000000000000000000000000000000000000000",
      "timestamp": "0x68cf1a3c",
      "gasLimit": "0x1c9c380",
      "gasUsed": "0x5a3c",
      "baseFeePerGas": "0x4c4b40",
      "calls": [
        {
          "status": "0x1",
          "gasUsed": "0x5a3c",
          "maxUsedGas": "0x5a3c",
          "returnData": "0x0000000000000000000000000000000000000000000000000000000005f5e100",
          "logs": []
        }
      ]
    }
  ]
}
```

## Behaviour at `pending` on this node

Simulation runs against the pre-confirmed block's state, so it sees state left by transactions the
builder has already sequenced but that are not yet in a sealed block. The simulated blocks are
numbered from the latest block, and each one starts from the state the previous one left, on top
of the pre-confirmed state. After the Amsterdam fork their slot numbers also count on from the
latest block, one per simulated block, so they differ from the builder's slot when a slot was
missed. Each simulated block takes the block environment that follows the latest block, not the
flashblock's: after a block that another builder made, its `COINBASE` and
gas limit can differ from the flashblock's, and `PREVRANDAO` is 0 unless overridden. While the node
holds the next block executed for its consensus client but not yet canonical, the simulated blocks
follow that block instead.

Nothing is submitted and no state is persisted. To submit and wait for pre-confirmation instead,
use [`eth_sendRawTransactionSync`](./eth_sendRawTransactionSync.md).

## Errors

| Code | Message |
|---|---|
| `-32602` | Invalid params, such as an empty `blockStateCalls` array (`calls are empty.`) |
| `-38026` | `too many blocks`: more than the node allows, 256 by default |

A reverted call does not fail the request: its entry in `calls` has `status` `0x0` and an `error`
with code 3.
