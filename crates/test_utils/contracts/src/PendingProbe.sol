// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

/// Probes for state that only pending re-execution can produce.
contract PendingProbe {
    /// Set by `recordParentHash`, read back through the generated getter.
    bytes32 public parentHash;

    /// Persists `BLOCKHASH(number - 1)` so pending execution is observable from storage.
    function recordParentHash() public {
        parentHash = blockhash(block.number - 1);
    }

    /// Reports this account's code size, which reflects the bytecode a pending override carries.
    function codeSize() public view returns (uint256) {
        return address(this).code.length;
    }
}
