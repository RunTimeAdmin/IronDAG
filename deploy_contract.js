const { ethers } = require('ethers');

// SimpleStorage contract
const CONTRACT_ABI = [
    "constructor()",
    "function set(uint256 x) public",
    "function get() public view returns (uint256)",
    "function storedData() public view returns (uint256)"
];

const CONTRACT_BYTECODE = "0x608060405234801561001057600080fd5b50602a60008190555061017c806100286000396000f3fe608060405234801561001057600080fd5b50600436106100415760003560e01c806360fe47b1146100465780632a1afcd9146100625780636d4ce63c14610080575b600080fd5b610060600480360381019061005b91906100ec565b61009e565b005b61006a6100a8565b6040516100779190610128565b60405180910390f35b6100886100ae565b6040516100959190610128565b60405180910390f35b8060008190555050565b60005481565b60008054905090565b600080fd5b6000819050919050565b6100cc816100b9565b81146100d757600080fd5b50565b6000813590506100e9816100c3565b92915050565b600060208284031215610105576101046100b4565b5b6000610113848285016100da565b91505092915050565b610122816100b9565b82525050565b600060208201905061013d6000830184610119565b9291505056fea26469706673582212206c6d0f3e2e4e6e4e2e4e2e4e2e4e2e4e2e4e2e4e2e4e2e4e2e4e2e4e64736f6c63430008130033";

// Utility: enforce a max wait (ms) on async operations to avoid hanging forever
async function withTimeout(promise, label, ms = 300000) {
    return Promise.race([
        promise,
        new Promise((_, reject) =>
            setTimeout(() => reject(new Error(`${label} timed out after ${ms}ms`)), ms)
        ),
    ]);
}

async function main() {
    console.log('🚀 Deploying SimpleStorage Contract...\n');
    
    // Connect to local node
    const provider = new ethers.JsonRpcProvider('http://127.0.0.1:8545');
    
    // Test connection
    console.log('✅ Connected to http://127.0.0.1:8545');
    
    // Use funded test account: 0x7e5f4552091a69125d5dfcb7b8c2659029395bdf
    // Private key: 0x0000000000000000000000000000000000000000000000000000000000000001
    const privateKey = '0x0000000000000000000000000000000000000000000000000000000000000001';
    const wallet = new ethers.Wallet(privateKey, provider);
    console.log(`📝 Deployer address: ${wallet.address}\n`);
    
    // Check balance
    const balance = await provider.getBalance(wallet.address);
    console.log(`💰 Balance: ${ethers.formatEther(balance)} IDAG\n`);
    
    // Create contract factory
    const factory = new ethers.ContractFactory(CONTRACT_ABI, CONTRACT_BYTECODE, wallet);
    
    try {
        console.log('📤 Sending deployment transaction...');
        const contract = await factory.deploy();
        console.log(`⏳ Transaction hash: ${contract.deploymentTransaction().hash}`);
        
        console.log('⏳ Waiting for deployment (timeout 300s)...');
        await withTimeout(contract.waitForDeployment(), 'contract deployment', 300_000);
        
        const address = await contract.getAddress();
        console.log(`\n✅ Contract deployed at: ${address}`);
        
        // Test reading
        console.log('\n🔍 Testing contract...');
        const value = await contract.storedData();
        console.log(`   Initial value: ${value}`);
        
        // Test writing
        console.log('\n📝 Setting value to 100...');
        const tx = await contract.set(100);
        await withTimeout(tx.wait(), 'set() confirmation', 300_000);
        console.log('✅ Value set!');
        
        const newValue = await contract.storedData();
        console.log(`   New value: ${newValue}`);
        
        console.log('\n🎉 SUCCESS! Contract is working!\n');
        console.log(`Contract Address: ${address}`);
        
    } catch (e) {
        console.error('\n❌ Deployment failed:', e.message);
        if (e.data) console.error('Error data:', e.data);
        process.exit(1);
    }
}

main().catch(console.error);
