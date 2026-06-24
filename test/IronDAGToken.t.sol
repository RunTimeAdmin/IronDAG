// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

import "forge-std/Test.sol";
import "../contracts/IronDAGToken.sol";

contract IronDAGTokenTest is Test {
    IronDAGToken public token;
    address public owner;
    address public user1;
    address public user2;

    function setUp() public {
        owner = address(this);
        user1 = address(0x1);
        user2 = address(0x2);
        token = new IronDAGToken(1_000_000);
    }

    function test_deploy_totalSupply() public view {
        assertEq(token.totalSupply(), 1_000_000 * 10**18);
        assertEq(token.balanceOf(owner), 1_000_000 * 10**18);
    }

    function test_transfer() public {
        token.transfer(user1, 100 * 10**18);
        assertEq(token.balanceOf(owner), 1_000_000 * 10**18 - 100 * 10**18);
        assertEq(token.balanceOf(user1), 100 * 10**18);
    }

    function test_transfer_revertInsufficientBalance() public {
        vm.prank(user1);
        vm.expectRevert(IronDAGToken.InsufficientBalance.selector);
        token.transfer(user2, 1);
    }

    function test_transfer_revertInvalidRecipient() public {
        vm.expectRevert(IronDAGToken.InvalidRecipient.selector);
        token.transfer(address(0), 100);
    }

    function test_approve_and_transferFrom() public {
        token.approve(user1, 200 * 10**18);
        vm.prank(user1);
        token.transferFrom(owner, user2, 200 * 10**18);
        assertEq(token.balanceOf(owner), 1_000_000 * 10**18 - 200 * 10**18);
        assertEq(token.balanceOf(user2), 200 * 10**18);
        assertEq(token.allowance(owner, user1), 0);
    }

    function test_transferFrom_revertInsufficientAllowance() public {
        vm.prank(user1);
        vm.expectRevert(IronDAGToken.InsufficientAllowance.selector);
        token.transferFrom(owner, user2, 1);
    }

    function test_decreaseAllowance_revertBelowZero() public {
        token.approve(user1, 50);
        vm.expectRevert(IronDAGToken.DecreasedAllowanceBelowZero.selector);
        token.decreaseAllowance(user1, 100);
    }

    function test_burn() public {
        token.burn(100 * 10**18);
        assertEq(token.totalSupply(), 1_000_000 * 10**18 - 100 * 10**18);
        assertEq(token.balanceOf(owner), 1_000_000 * 10**18 - 100 * 10**18);
    }

    function test_burn_revertInsufficientBalance() public {
        vm.prank(user1);
        vm.expectRevert(IronDAGToken.InsufficientBalance.selector);
        token.burn(1);
    }
}
