# Simulate at `pending`

Example 5 of 7 in [Examples](./README.md). The commands use `rpc` and the variables of
[Before you start](./README.md#before-you-start). Run them while the transfer of
[Send a transaction and wait for its pre-confirmation](./send-and-wait.md) is pre-confirmed.

`TO` holds its 1 ETH only in pre-confirmed state. An estimate without a block parameter runs at
`pending`, so it can spend it; the same estimate at `latest` cannot:

```sh
rpc eth_estimateGas "[{\"from\": \"$TO\", \"to\": \"$FROM\", \"value\": \"0x16345785d8a0000\"}]"
rpc eth_estimateGas "[{\"from\": \"$TO\", \"to\": \"$FROM\", \"value\": \"0x16345785d8a0000\"}, \"latest\"]"
```

```json
{ "jsonrpc": "2.0", "id": 1, "result": "0x5208" }
{ "jsonrpc": "2.0", "id": 1, "error": { "code": -32003, "message": "EVM error: OutOfFunds" } }
```

So an approval that is pre-confirmed counts in the estimate of the swap after it. web3.py sends
`latest` unless told otherwise; see [Client libraries](../client-libraries.md#gas-estimates) and
[`eth_estimateGas`](../ethereum-json-rpc-api/eth_estimateGas.md).

`eth_simulateV1` runs calls on top of the pre-confirmed state. Here, `increment()` (selector
`0xd09de08a`), then `number()`:

```sh
rpc eth_simulateV1 "[{\"blockStateCalls\": [{\"calls\": [
  {\"from\": \"$TO\", \"to\": \"$COUNTER\", \"data\": \"0xd09de08a\"},
  {\"from\": \"$TO\", \"to\": \"$COUNTER\", \"data\": \"0x8381f58a\"}]}]}, \"pending\"]"
```

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": [
    {
      "number": "0x5d",
      "stateRoot": "0x0000000000000000000000000000000000000000000000000000000000000000",
      "calls": [
        { "returnData": "0x", "logs": [], "gasUsed": "0x670e", "status": "0x1" },
        {
          "returnData": "0x0000000000000000000000000000000000000000000000000000000000000004",
          "logs": [],
          "gasUsed": "0x5bc0",
          "status": "0x1"
        }
      ]
    }
  ]
}
```

The simulation started from the pending `number()` of 3, so it returns 4. The simulated block is
numbered like the pending block, the latest plus one. Nothing is sent. Each simulated block takes
the block environment that follows the latest block, not the flashblock's; see
[`eth_simulateV1`](../flashblocks-api/eth_simulateV1.md).

---

Next: [Wait for the seal](./wait-for-the-seal.md).
