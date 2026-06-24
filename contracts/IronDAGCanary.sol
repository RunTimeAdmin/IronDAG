// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

contract IronDAGCanary {
    error Unauthorized();

    uint256 public value;
    address public owner;

    event ValueChanged(uint256 newValue, address changedBy);

    constructor() {
        owner = msg.sender;
        value = 888; // Lucky number test
    }

    modifier onlyOwner() {
        if (msg.sender != owner) revert Unauthorized();
        _;
    }

    function setValue(uint256 _newValue) public onlyOwner {
        value = _newValue;
        emit ValueChanged(_newValue, msg.sender);
    }

    function getBlockNumber() public view returns (uint256) {
        return block.number;
    }
}
