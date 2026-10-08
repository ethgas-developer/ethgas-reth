# web3.py 7 (Python)

One of the [Examples](./README.md). The program reads at `pending`, sends 1 wei and waits for its
pre-confirmation, then prints three stream messages. It takes `RPC`, `WS`, `PRIVATE_KEY` and `TO`
from the environment; see [Before you start](./README.md#before-you-start).

Tested with web3.py 7.16.0 (eth-account 0.14.0) on Python 3.14.7.

web3.py has no method for `eth_sendRawTransactionSync`: `make_request` makes the call. web3.py 7
cannot read the full form of `newFlashblockTransactions`, so the program streams hashes.

```python
import asyncio
import os

from eth_account import Account
from web3 import AsyncWeb3, Web3, WebSocketProvider

w3 = Web3(Web3.HTTPProvider(os.environ["RPC"]))
account = Account.from_key(os.environ["PRIVATE_KEY"])

# Read at pending
block = w3.eth.get_block("pending")
nonce = w3.eth.get_transaction_count(account.address, "pending")
print("pending block", block["number"], "with", len(block["transactions"]), "transactions; nonce", nonce)

# Send, and wait for the pre-confirmation
tip = w3.eth.max_priority_fee
tx = {
    "to": Web3.to_checksum_address(os.environ["TO"]), "value": 1, "nonce": nonce, "gas": 21000,
    "chainId": w3.eth.chain_id, "maxPriorityFeePerGas": tip, "maxFeePerGas": 2 * block["baseFeePerGas"] + tip,
}
raw = account.sign_transaction(tx).raw_transaction.to_0x_hex()
receipt = w3.provider.make_request("eth_sendRawTransactionSync", [raw, 5000])["result"]
print("pre-confirmed" if int(receipt["blockHash"], 16) == 0 else "sealed", "in block", int(receipt["blockNumber"], 16))


# Stream: the hash of each transaction the builder sequences
async def stream():
    async with AsyncWeb3(WebSocketProvider(os.environ["WS"])) as ws:
        await ws.eth.subscribe("newFlashblockTransactions")
        seen = 0
        async for message in ws.socket.process_subscriptions():
            print("sequenced", message["result"].to_0x_hex())
            seen += 1
            if seen == 3:
                break


asyncio.run(stream())
```

Its output on the test network:

```text
pending block 125 with 254 transactions; nonce 7
pre-confirmed in block 125
sequenced 0xff3247061e7b6dcf008ca66d4395d7fc5da3a1da7fadb628e348df42c99336f5
sequenced 0xf6961c0064d89653f0b064e5a1ad7b5cc957dad090bf33b1e73258e5ffb4e628
sequenced 0xefdec4c010d08526abe10994667035dbe8283a840d49e4cc3422aeba415e1d2d
```

What the library does with this node's answers elsewhere is on
[Client libraries](../client-libraries.md).
