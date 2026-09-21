# ethgas-reth documentation

`ethgas-node` is a [reth](https://github.com/paradigmxyz/reth)-based Ethereum execution client with
flashblocks support. It changes what the `pending` block tag means: instead of a block the node
invents from its own mempool, `pending` is the block the builder is currently assembling.
