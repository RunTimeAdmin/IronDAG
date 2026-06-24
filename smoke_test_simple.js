#!/usr/bin/env node
const { ethers } = require('ethers');

// Connect to local node
const provider = new ethers.JsonRpcProvider('http://127.0.0.1:8545');

// Test wallet with deterministic private key (only for testing!)
// Address: 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266
const TEST_PRIVATE_KEY = '0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80';
const wallet = new ethers.Wallet(TEST_PRIVATE_KEY, provider);

async function main() {
    const contractAddr = '0x1000000000000000000000000000000000000001';
    
    console.log('🧪 SMOKE TEST - EVM Storage Persistence\n');
    console.log('Test wallet address:', wallet.address);
    
    // Check current block number
    try {
        const blockNumber = await provider.getBlockNumber();
        console.log('Current block:', blockNumber);
    } catch (e) {
        console.log('Could not get block number:', e.message);
    }
    
    // Check balance of miner address (which should have tokens)
    const minerAddr = '0x0101010101010101010101010101010101010101';
    try {
        const minerBalance = await provider.getBalance(minerAddr);
        console.log('Miner balance:', ethers.formatUnits(minerBalance, 18), 'IDAG');
    } catch (e) {
        console.log('Could not fetch miner balance:', e.message);
    }
    
    // Call getValue() to read current storage
    console.log('\nStep 1: Calling getValue() to read current storage...');
    const getValueCalldata = '0x20965255';
    
    try {
        const result = await provider.call({
            to: contractAddr,
            data: getValueCalldata
        });
        
        const value = BigInt(result);
        console.log('✅ getValue() returned:', value.toString());
        
        if (value === 0n) {
            console.log('   (Storage slot is empty - contract may not be deployed)');
        }
    } catch (error) {
        console.error('❌ getValue failed:', error.message);
    }
    
    // Simulate setValue(999) via eth_call (read-only, doesn't persist)
    console.log('\nStep 2: Simulating setValue(999) via eth_call...');
    const setValueCalldata = '0x60fe47b1' + (999).toString(16).padStart(64, '0');
    
    try {
        const result = await provider.call({
            to: contractAddr,
            data: setValueCalldata
        });
        console.log('✅ eth_call setValue simulation result:', result || '0x (empty - success)');
    } catch (error) {
        console.error('❌ setValue simulation failed:', error.message);
    }
    
    console.log('\n✅ Smoke test complete');
    console.log('\nNote: To persist state changes, you need to send a signed transaction.');
    console.log('The miner address receives block rewards but the key is internal to the node.');
}

main().catch(console.error);
