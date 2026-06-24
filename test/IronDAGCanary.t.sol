// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

import "forge-std/Test.sol";
import "../contracts/IronDAGCanary.sol";

contract IronDAGCanaryTest is Test {
    IronDAGCanary public canary;
    address public owner;
    address public stranger;

    function setUp() public {
        owner = address(this);
        stranger = address(0x1);
        canary = new IronDAGCanary();
    }

    function test_initialValue() public view {
        assertEq(canary.value(), 888);
        assertEq(canary.owner(), owner);
    }

    function test_setValue_asOwner() public {
        canary.setValue(999);
        assertEq(canary.value(), 999);
    }

    function test_setValue_revertUnauthorized() public {
        vm.prank(stranger);
        vm.expectRevert(IronDAGCanary.Unauthorized.selector);
        canary.setValue(123);
    }

    function test_getBlockNumber() public view {
        assertEq(canary.getBlockNumber(), block.number);
    }
}
