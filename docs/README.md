# ethgas-reth documentation

`ethgas-node` is a [reth](https://github.com/paradigmxyz/reth)-based Ethereum execution client with
flashblocks support. It changes what the `pending` block tag means: instead of a block the node
invents from its own mempool, `pending` is the block the builder is currently assembling.

- [RPC overview](./rpc-overview.md): every method, and what it does with `pending`
- [Flashblocks API](./flashblocks-api/README.md): the methods and subscriptions only this node has
- [Examples](./examples/README.md): requests and answers for the flashblocks-aware methods, one
  example a page, with `curl` and `websocat`, and in ethers, viem, web3.py and alloy (Rust)
- [Client libraries](./client-libraries.md): what ethers, viem, web3.py, alloy and MetaMask do with
  this node's answers
- [Run a node](./run-a-node.md)
