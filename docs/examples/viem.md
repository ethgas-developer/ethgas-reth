# viem 2 (JavaScript)

One of the [Examples](./README.md). The program reads at `pending`, sends 1 wei and waits for its
pre-confirmation, then prints three stream messages. It takes `RPC`, `WS`, `PRIVATE_KEY` and `TO`
from the environment; see [Before you start](./README.md#before-you-start).

Tested with viem 2.57.3 on Node.js 24.21.0. Save it as `viem.mjs`, because it uses ES modules
and top-level `await`.

`sendTransactionSync` signs locally and calls `eth_sendRawTransactionSync`. The WebSocket
transport's `subscribe` takes any subscription.

```js
import { createPublicClient, createWalletClient, defineChain, http, webSocket, zeroHash } from "viem";
import { privateKeyToAccount } from "viem/accounts";

const chain = defineChain({
  id: 3151908,
  name: "ethgas-devnet",
  nativeCurrency: { name: "Ether", symbol: "ETH", decimals: 18 },
  rpcUrls: { default: { http: [process.env.RPC], webSocket: [process.env.WS] } },
});
const account = privateKeyToAccount(process.env.PRIVATE_KEY);
const client = createPublicClient({ chain, transport: http() });
const wallet = createWalletClient({ account, chain, transport: http() });

// Read at pending
const block = await client.getBlock({ blockTag: "pending" });
const nonce = await client.getTransactionCount({ address: account.address, blockTag: "pending" });
console.log("pending block", Number(block.number), "with", block.transactions.length, "transactions; nonce", nonce);

// Send, and wait for the pre-confirmation
const receipt = await wallet.sendTransactionSync({ to: process.env.TO, value: 1n, timeout: 5000 });
console.log(receipt.blockHash === zeroHash ? "pre-confirmed" : "sealed", "in block", Number(receipt.blockNumber));

// Stream: one message per flashblock
const ws = createPublicClient({ chain, transport: webSocket() });
let seen = 0;
const { unsubscribe } = await ws.transport.subscribe({
  params: ["newFlashblocks"],
  onData: async ({ result }) => {
    console.log("block", Number(result.number), "now has", result.transactions.length, "transactions");
    if (++seen === 3) {
      await unsubscribe();
      (await ws.transport.getRpcClient()).close();
    }
  },
});
```

Its output on the test network:

```text
pending block 125 with 151 transactions; nonce 6
pre-confirmed in block 125
block 125 now has 203 transactions
block 125 now has 228 transactions
block 125 now has 254 transactions
```

What the library does with this node's answers elsewhere is on
[Client libraries](../client-libraries.md).
