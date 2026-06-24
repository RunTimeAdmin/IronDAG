// Storage persistence diagnostic test
const { ethers } = require('ethers');

async function main() {
    const provider = new ethers.JsonRpcProvider('http://127.0.0.1:8545');
    
    // Contract from previous deployment
    // Update this address if deploying fresh
    const contractAddress = '0x2946259E0334f33A064106302415aD3391BeD384';
    
    console.log('=== Storage Persistence Diagnostic ===\n');
    
    // 1. Check eth_getStorageAt directly (bypasses EVM execution)
    console.log('1. Direct storage read via eth_getStorageAt:');
    const storageSlot0 = await provider.getStorage(contractAddress, 0);
    console.log(`   Slot 0 raw: ${storageSlot0}`);
    console.log(`   As number: ${BigInt(storageSlot0)}\n`);
    
    // 2. Check via eth_call (get function)
    console.log('2. Storage read via eth_call (get()):');
    const getCalldata = '0x6d4ce63c'; // get() selector
    const result = await provider.call({
        to: contractAddress,
        data: getCalldata
    });
    console.log(`   get() result: ${result}`);
    console.log(`   As number: ${BigInt(result)}\n`);
    
    // 3. Check contract code exists
    console.log('3. Contract code check:');
    const code = await provider.getCode(contractAddress);
    console.log(`   Code length: ${(code.length - 2) / 2} bytes`);
    console.log(`   Code exists: ${code.length > 2 ? 'YES' : 'NO'}\n`);
    
    // 4. Send a new set transaction and watch
    console.log('4. Sending new set(777) transaction...');
    const privateKey = '0x0000000000000000000000000000000000000000000000000000000000000001';
    const wallet = new ethers.Wallet(privateKey, provider);
    
    // set(777) calldata: 0x60fe47b1 + 777 as 256-bit
    const setCalldata = '0x60fe47b1' + (777).toString(16).padStart(64, '0');
    
    const tx = await wallet.sendTransaction({
        to: contractAddress,
        data: setCalldata,
        gasLimit: 100000
    });
    console.log(`   Tx hash: ${tx.hash}`);
    
    console.log('   Waiting for confirmation...');
    const receipt = await tx.wait();
    console.log(`   Block: ${receipt.blockNumber}, Gas: ${receipt.gasUsed}\n`);
    
    // 5. Read again immediately
    console.log('5. Read immediately after set:');
    const storageAfter = await provider.getStorage(contractAddress, 0);
    console.log(`   Slot 0 via eth_getStorageAt: ${storageAfter} = ${BigInt(storageAfter)}`);
    
    const resultAfter = await provider.call({
        to: contractAddress,
        data: getCalldata
    });
    console.log(`   Slot 0 via eth_call get(): ${resultAfter} = ${BigInt(resultAfter)}\n`);
    
    // 6. Summary
    console.log('=== Diagnosis ===');
    const finalValue = BigInt(storageAfter);
    if (finalValue === 777n) {
        console.log('✅ Storage persistence is WORKING');
    } else if (finalValue === 42n) {
        console.log('❌ Storage NOT persisting - still at initial value');
        console.log('   Root cause: BundleState not capturing SSTORE in revm 33.1');
    } else if (finalValue === 100n) {
        console.log('⚠️  Old value (100) persisted, new set(777) did not');
    } else {
        console.log(`⚠️  Unexpected value: ${finalValue}`);
    }
}

main().catch(console.error);
