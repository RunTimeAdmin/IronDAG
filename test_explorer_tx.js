const { ethers } = require('ethers');

// Default to local node, can override with env var or command line
const RPC_URL = process.env.RPC_URL || process.argv[2] || 'http://127.0.0.1:8545';

// Test wallet with private key 0x01 - has 1000 MSHW on testnet
const TEST_PRIVATE_KEY = '0x0000000000000000000000000000000000000000000000000000000000000001';

async function sendTestTransactions() {
    console.log('Connecting to:', RPC_URL);
    
    const provider = new ethers.JsonRpcProvider(RPC_URL);
    const wallet = new ethers.Wallet(TEST_PRIVATE_KEY, provider);
    
    console.log('Test wallet address:', wallet.address);
    
    // Check balance
    const balance = await provider.getBalance(wallet.address);
    console.log('Balance:', ethers.formatEther(balance), 'MSHW');
    
    // Get current nonce
    let nonce = await provider.getTransactionCount(wallet.address);
    console.log('Current nonce:', nonce);
    
    // Get current block
    const blockNumber = await provider.getBlockNumber();
    console.log('Current block:', blockNumber);
    
    if (balance === 0n) {
        console.log('\nWallet has no balance!');
        return;
    }
    
    // Send 3 test transactions with explicit nonces
    console.log('\nSending test transactions...');
    
    for (let i = 1; i <= 3; i++) {
        try {
            const tx = await wallet.sendTransaction({
                to: '0x' + '00'.repeat(19) + i.toString(16).padStart(2, '0'),
                value: ethers.parseEther('0.1'),
                gasLimit: 21000n,
                nonce: nonce++, // Explicitly set and increment nonce
            });
            console.log(`TX ${i} sent with nonce ${nonce - 1}:`, tx.hash);
            
            console.log(`   Waiting for confirmation...`);
            const receipt = await tx.wait(1, 30000);
            console.log(`TX ${i} confirmed in block:`, receipt.blockNumber);
        } catch (error) {
            console.error(`TX ${i} failed:`, error.message);
        }
    }
    
    console.log('\nDone!');
}

sendTestTransactions().catch(console.error);
