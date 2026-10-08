# ethers 6 (JavaScript)

One of the [Examples](./README.md). The program reads at `pending`, sends 1 wei and waits for its
pre-confirmation, then prints three stream messages. It takes `RPC`, `WS`, `PRIVATE_KEY` and `TO`
from the environment; see [Before you start](./README.md#before-you-start).

Tested with ethers 6.17.0 on Node.js 24.21.0. Save it as `ethers.mjs`, because it uses ES
modules and top-level `await`.

ethers has no method for `eth_sendRawTransactionSync` and no type for the flashblocks
subscriptions: `provider.send` makes the call, and a `SocketSubscriber` subclass makes the
subscription.

```js
import { JsonRpcProvider, SocketSubscriber, Wallet, WebSocketProvider, ZeroHash } from "ethers";

const provider = new JsonRpcProvider(process.env.RPC);
const wallet = new Wallet(process.env.PRIVATE_KEY, provider);

// Read at pending
const block = await provider.getBlock("pending");
const nonce = await provider.getTransactionCount(wallet.address, "pending");
console.log("pending block", block.number, "with", block.transactions.length, "transactions; nonce", nonce);

// Send, and wait for the pre-confirmation
const raw = await wallet.signTransaction(await wallet.populateTransaction({ to: process.env.TO, value: 1n }));
const receipt = await provider.send("eth_sendRawTransactionSync", [raw, 5000]);
console.log(receipt.blockHash === ZeroHash ? "pre-confirmed" : "sealed", "in block", Number(receipt.blockNumber));
provider.destroy();

// Stream: one message per flashblock
class Flashblocks extends SocketSubscriber {
  seen = 0;
  constructor(ws) {
    super(ws, ["newFlashblocks"]);
  }
  async _emit(ws, block) {
    console.log("block", Number(block.number), "now has", block.transactions.length, "transactions");
    if (++this.seen === 3) ws.destroy();
  }
}
new Flashblocks(new WebSocketProvider(process.env.WS)).start();
```

Its output on the test network:

```text
pending block 124 with 251 transactions; nonce 5
pre-confirmed in block 124
block 124 now has 304 transactions
block 125 now has 127 transactions
block 125 now has 151 transactions
```

What the library does with this node's answers elsewhere is on
[Client libraries](../client-libraries.md).
