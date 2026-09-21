# eth_simulateV1

Simulates one or more bundles of calls against pre-confirmed state, without submitting anything.

| | |
|---|---|
| Flashblocks `pending` | **Yes** — simulates against the flashblock being built |
| Transport | HTTP or WebSocket |

## Parameters

| Name | Type | Required | Description |
|---|---|---|---|
| `payload` | object | yes | The simulation. See below |
| `block` | string | no | Block number in hex, or a tag. Pass `"pending"` to simulate against pre-confirmed state |

`payload` fields:

| Field | Type | Description |
|---|---|---|
| `blockStateCalls` | array | One entry per simulated block. Each holds `calls`, and optionally `stateOverrides` and `blockOverrides` |
| `traceTransfers` | boolean | Emit ETH transfers as logs. Defaults to `false` |
| `validation` | boolean | Enforce nonce and balance checks. Defaults to `false` |

Each entry in `blockStateCalls`:

| Field | Type | Description |
|---|---|---|
| `calls` | array | Transaction request objects to execute in order |
| `stateOverrides` | object | Per-address overrides of `balance`, `nonce`, `code` or `state` |
| `blockOverrides` | object | Overrides of block fields such as `number` and `time` |

## Returns

An array of simulated blocks. Each carries the usual block fields plus a `calls` array, one entry
per call, with `status`, `gasUsed`, `returnData`, `logs`, and `error` where the call failed.

`stateRoot` on a simulated block is `0x000…0`. Simulation does not commit to a trie.

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
              "from": "0xd3CdA913deB6f4967b2Ef66ae97DE114a83bcc01",
              "to": "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48",
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
      "hash": "0x0000000000000000000000000000000000000000000000000000000000000000",
      "stateRoot": "0x0000000000000000000000000000000000000000000000000000000000000000",
      "timestamp": "0x68cf1a3c",
      "gasLimit": "0x1c9c380",
      "gasUsed": "0x5a3c",
      "baseFeePerGas": "0x3b9aca00",
      "calls": [
        {
          "status": "0x1",
          "gasUsed": "0x5a3c",
          "returnData": "0x0000000000000000000000000000000000000000000000000000000005f5e100",
          "logs": []
        }
      ]
    }
  ]
}
```

## Behaviour at `pending` on this node

Simulation runs against the flashblock being built, so it sees state left by transactions the
builder has already sequenced but that are not yet in a sealed block.

Nothing is submitted and no state is persisted. To submit and wait for pre-confirmation instead,
use [`eth_sendRawTransactionSync`](./eth_sendRawTransactionSync.md).

## Errors

| Code | Message |
|---|---|
| `-32000` | `execution reverted` — the revert reason, where one exists, is in the error `data` |
| `-32602` | Invalid params, including a `blockStateCalls` array longer than the node allows |
