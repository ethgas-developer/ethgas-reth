//! Proposer payment shapes the simulator recognises besides a plain transfer.

use alloy_primitives::{Address, B256, U256, address, b256};
use alloy_sol_types::{SolCall, sol};

/// The `PaymentForwarder` contract (helix `contracts/README.md`). A payment routed through it is
/// only valid in the block it was signed for.
pub const PAYMENT_FORWARDER: Address = address!("0xFEEEEEE44046c3f61a8CC081E0918eF0de0a7ffC");

/// Runtime deployed at [`PAYMENT_FORWARDER`]. A value call to that address on a
/// chain without it succeeds while crediting the address itself, so a payment is
/// only recognised where the runtime is present.
pub const PAYMENT_FORWARDER_CODE_HASH: B256 =
    b256!("0xd9f5db49d3c0a174c39701485406cd78d01dd27f73b7ef7b883e5f69d8103220");

/// Recipient in a [`PAYMENT_FORWARDER`] call, whose calldata is a 4 byte
/// timestamp followed by the 20 byte recipient.
#[inline(always)]
pub fn payment_forwarder_recipient(input: &[u8]) -> Option<Address> {
    (input.len() >= 24).then(|| Address::from_slice(&input[4..24]))
}

sol! {
    /// Gnosis Safe entry point the merge-builder payment engine calls
    /// (helix `crates/builder/src/engine/payment.rs`) to delegatecall `multiSend`.
    function execTransaction(
        address to,
        uint256 value,
        bytes data,
        uint8 operation,
        uint256 safeTxGas,
        uint256 baseGas,
        uint256 gasPrice,
        address gasToken,
        address refundReceiver,
        bytes signatures
    ) external returns (bool);

    /// Gnosis `MultiSendCallOnly` entry point, delegatecalled by `execTransaction`.
    function multiSend(bytes transactions) external payable;
}

/// The Safe `operation` value of a delegatecall.
pub const SAFE_DELEGATECALL: u8 = 1;

/// Sums the value a Safe `execTransaction` -> `multiSend` delegatecall's batched
/// calls pay `recipient` (zero on any decode failure or shape mismatch: not a
/// delegatecall, not a `multiSend` payload, no matching entry). Best-effort
/// recognition, not a signature/authenticity check: the caller must already know
/// the transaction actually succeeded on-chain.
pub fn multisend_paid_amount(input: &[u8], recipient: Address) -> U256 {
    let Ok(exec) = execTransactionCall::abi_decode(input) else { return U256::ZERO };
    if exec.operation != SAFE_DELEGATECALL {
        return U256::ZERO;
    }
    let Ok(multisend) = multiSendCall::abi_decode(&exec.data) else { return U256::ZERO };
    multisend_entries(&multisend.transactions)
        .filter(|(to, _)| *to == recipient)
        .fold(U256::ZERO, |acc, (_, value)| acc + value)
}

/// Iterates a Safe `multiSend` packed payload: per entry, `[1B operation][20B
/// to][32B value][32B dataLength][dataLength B data]`. Stops (yields no more
/// entries) on any length that doesn't fit the remaining bytes, rather than
/// panicking on malformed/adversarial input.
pub fn multisend_entries(transactions: &[u8]) -> impl Iterator<Item = (Address, U256)> + '_ {
    const ENTRY_HEADER_LEN: usize = 1 + 20 + 32 + 32;
    let mut offset = 0usize;
    std::iter::from_fn(move || {
        if transactions.len().saturating_sub(offset) < ENTRY_HEADER_LEN {
            return None;
        }
        offset += 1; // operation: irrelevant for a payment check
        let to = Address::from_slice(&transactions[offset..offset + 20]);
        offset += 20;
        let value = U256::from_be_slice(&transactions[offset..offset + 32]);
        offset += 32;
        let data_len: usize =
            U256::from_be_slice(&transactions[offset..offset + 32]).try_into().ok()?;
        offset += 32;
        if transactions.len().saturating_sub(offset) < data_len {
            return None;
        }
        offset += data_len;
        Some((to, value))
    })
}

#[cfg(test)]
mod tests {
    use alloy_primitives::Bytes;

    use super::*;

    /// A Safe `multiSend` payload of plain transfers.
    fn multisend_payload(entries: &[(Address, U256)]) -> Vec<u8> {
        let mut payload = Vec::new();
        for (to, value) in entries {
            payload.push(0u8); // operation = CALL
            payload.extend_from_slice(to.as_slice());
            payload.extend_from_slice(&value.to_be_bytes::<32>());
            payload.extend_from_slice(&U256::ZERO.to_be_bytes::<32>()); // data length
        }
        payload
    }

    /// An unsigned Safe `execTransaction` that runs `multiSend` over `entries` with `operation`.
    fn exec_transaction_calldata(operation: u8, entries: &[(Address, U256)]) -> Vec<u8> {
        let multisend = multiSendCall { transactions: multisend_payload(entries).into() };
        execTransactionCall {
            to: Address::ZERO,
            value: U256::ZERO,
            data: multisend.abi_encode().into(),
            operation,
            safeTxGas: U256::ZERO,
            baseGas: U256::ZERO,
            gasPrice: U256::ZERO,
            gasToken: Address::ZERO,
            refundReceiver: Address::ZERO,
            signatures: Bytes::new(),
        }
        .abi_encode()
    }

    /// Only a delegatecall batch pays, and only its entries to the recipient count.
    #[test]
    fn multisend_paid_amount_sums_delegatecall_entries_to_the_recipient() {
        let proposer = address!("0x2222222222222222222222222222222222222222");
        let other = address!("0x3333333333333333333333333333333333333333");
        let entries =
            [(other, U256::from(100)), (proposer, U256::from(42)), (proposer, U256::from(8))];

        let delegatecall = exec_transaction_calldata(SAFE_DELEGATECALL, &entries);
        assert_eq!(multisend_paid_amount(&delegatecall, proposer), U256::from(50));
        let call = exec_transaction_calldata(0, &entries);
        assert_eq!(multisend_paid_amount(&call, proposer), U256::ZERO);
    }

    #[test]
    fn multisend_entries_stops_on_truncated_payload() {
        let to = address!("0x2222222222222222222222222222222222222222");
        let mut payload = multisend_payload(&[(to, U256::from(42))]);
        payload.truncate(payload.len() - 1); // cut into the last entry's data-length field

        assert_eq!(multisend_entries(&payload).count(), 0);
    }
}
