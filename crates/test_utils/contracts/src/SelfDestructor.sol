// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

/// A contract that destroys itself in its creating transaction or in a later one.
contract SelfDestructor {
    /// Destroys the contract during construction when `onCreate` is set. After Cancun
    /// (EIP-6780) that is the only case in which SELFDESTRUCT deletes the account.
    constructor(address payable beneficiary, bool onCreate) payable {
        if (onCreate) {
            selfdestruct(beneficiary);
        }
    }

    /// Sends the balance to `beneficiary`. After Cancun the code and storage stay.
    function destroy(address payable beneficiary) external {
        selfdestruct(beneficiary);
    }
}
