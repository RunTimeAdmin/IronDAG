// SPDX-License-Identifier: MIT
pragma solidity 0.8.20;

/**
 * @title IronDAGToken (IDAG)
 * @dev ERC-20 Graduation Exam - validates full smart contract platform
 * 
 * Tests validated:
 * 1. Contract deployment
 * 2. Balance storage persistence  
 * 3. Transfer execution
 * 4. eth_call queries (balanceOf, totalSupply)
 * 5. Event emission
 */
contract IronDAGToken {
    error InsufficientBalance();
    error InvalidRecipient();
    error InsufficientAllowance();
    error DecreasedAllowanceBelowZero();

    string public constant name = "IronDAG Token";
    string public constant symbol = "IDAG";
    uint8 public constant decimals = 18;

    uint256 public totalSupply;
    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;

    event Transfer(address indexed from, address indexed to, uint256 value);
    event Approval(address indexed owner, address indexed spender, uint256 value);

    constructor(uint256 _initialSupply) {
        totalSupply = _initialSupply * 10 ** decimals;
        balanceOf[msg.sender] = totalSupply;
        emit Transfer(address(0), msg.sender, totalSupply);
    }

    function transfer(address to, uint256 amount) public returns (bool) {
        if (balanceOf[msg.sender] < amount) revert InsufficientBalance();
        if (to == address(0)) revert InvalidRecipient();

        balanceOf[msg.sender] -= amount;
        balanceOf[to] += amount;
        emit Transfer(msg.sender, to, amount);
        return true;
    }

    function approve(address spender, uint256 amount) public returns (bool) {
        allowance[msg.sender][spender] = amount;
        emit Approval(msg.sender, spender, amount);
        return true;
    }

    function transferFrom(address from, address to, uint256 amount) public returns (bool) {
        if (balanceOf[from] < amount) revert InsufficientBalance();
        if (allowance[from][msg.sender] < amount) revert InsufficientAllowance();
        if (to == address(0)) revert InvalidRecipient();

        balanceOf[from] -= amount;
        balanceOf[to] += amount;
        allowance[from][msg.sender] -= amount;
        emit Transfer(from, to, amount);
        return true;
    }

    function increaseAllowance(address spender, uint256 addedValue) public returns (bool) {
        allowance[msg.sender][spender] += addedValue;
        emit Approval(msg.sender, spender, allowance[msg.sender][spender]);
        return true;
    }

    function decreaseAllowance(address spender, uint256 subtractedValue) public returns (bool) {
        if (allowance[msg.sender][spender] < subtractedValue) revert DecreasedAllowanceBelowZero();
        allowance[msg.sender][spender] -= subtractedValue;
        emit Approval(msg.sender, spender, allowance[msg.sender][spender]);
        return true;
    }

    function burn(uint256 amount) public returns (bool) {
        if (balanceOf[msg.sender] < amount) revert InsufficientBalance();

        balanceOf[msg.sender] -= amount;
        totalSupply -= amount;
        emit Transfer(msg.sender, address(0), amount);
        return true;
    }
}
