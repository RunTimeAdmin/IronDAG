# Deploying Smart Contracts on IronDAG

## Prerequisites

### Required Tools

- **MetaMask** browser extension or mobile app
- **Node.js** 16+ and npm/yarn
- **ethers.js** v5 or v6
- **Solidity compiler** (solc) or **Hardhat/Foundry**

### Install Dependencies

```bash
npm install ethers
```

Or with yarn:
```bash
yarn add ethers
```

---

## Network Configuration

### Chain ID

IronDAG uses **Chain ID 1337** by default (configurable via `--chain-id`).

### RPC URL

| Environment | URL |
|-------------|-----|
| Local node | `http://localhost:8546` |
| Testnet | `https://rpc.testnet.irondag.io` |
| With Nginx proxy | `https://rpc.irondag.io` |

### Block Explorer

- **Local**: Not available (use RPC directly)
- **Testnet**: `https://explorer.testnet.irondag.io`

### Network Parameters

| Parameter | Value |
|-----------|-------|
| Chain ID | 1337 (0x539) |
| Currency Symbol | IDAG |
| Currency Decimals | 18 |
| Block Time | 1-10 seconds (BraidCore) |
| Gas Limit | 30,000,000 per block |
| Default Gas Price | 20 gwei |

---

## MetaMask Configuration

### Add IronDAG Network

1. Open MetaMask
2. Click network dropdown → "Add Network"
3. Click "Add a network manually"
4. Enter details:

| Field | Value |
|-------|-------|
| Network Name | IronDAG Testnet |
| RPC URL | http://localhost:8546 (or your node) |
| Chain ID | 1337 |
| Currency Symbol | IDAG |
| Block Explorer URL | (optional) |

5. Click "Save"

### Get Testnet Tokens

If running a local node with test mode:

```bash
# The genesis allocation includes pre-funded addresses
# Default miner address: 0x0101010101010101010101010101010101010101
```

Or use the faucet method (test builds only):

```javascript
const provider = new ethers.JsonRpcProvider('http://localhost:8546');

// Request faucet tokens
await provider.send('irondag_faucet', [
  '0xYourAddress',
  '0x3635C9ADC5DEA00000' // 1000 IDAG
]);
```

---

## Supported EVM Features

### EVM Configuration

IronDAG uses **SputnikVM** (evm crate) with **Shanghai** fork configuration:

- **Config**: `Config::shanghai()`
- **EVM Revision**: Shanghai (supports PUSH0 opcode from EIP-3855)
- **Precompiles**: Standard Ethereum precompiles

### Supported Opcodes

All Shanghai fork opcodes are supported, including:

| Opcode | Description |
|--------|-------------|
| PUSH0 | Push zero onto stack (EIP-3855) |
| CREATE2 | Create contract with deterministic address |
| STATICCALL | Static contract call |
| DELEGATECALL | Delegate call |
| REVERT | Revert with reason |
| RETURNDATACOPY | Copy return data |
| RETURNDATASIZE | Get return data size |

### Gas Model

- Base transaction cost: 21,000 gas
- Per-byte data cost: 16 gas (non-zero), 4 gas (zero)
- Contract creation: Additional 32,000 gas
- Storage operations: 20,000 gas (SSTORE)

### Limitations

- No EIP-1559 fee market (uses legacy gas price)
- No blob transactions (EIP-4844)
- Verkle trees optional (not default)

---

## Using MetaMask

### Connect to DApp

```javascript
// Check if MetaMask is installed
if (typeof window.ethereum !== 'undefined') {
  console.log('MetaMask is installed!');
}

// Request account access
const accounts = await window.ethereum.request({
  method: 'eth_requestAccounts'
});
const account = accounts[0];
```

### Send Transaction

```javascript
const transactionParameters = {
  to: '0xRecipientAddress',
  from: account,
  value: '0x29a2241af62c0000', // 3 IDAG in wei
  gas: '0x5208', // 21000
  gasPrice: '0x4a817c800', // 20 gwei
};

const txHash = await window.ethereum.request({
  method: 'eth_sendTransaction',
  params: [transactionParameters],
});
```

### Deploy Contract via MetaMask

```javascript
// Contract bytecode (from compilation)
const bytecode = '0x608060405234801561001057600080fd5b50...';

// Constructor ABI (if any)
const abi = [
  {
    "inputs": [],
    "name": "get",
    "outputs": [{"internalType": "uint256", "name": "", "type": "uint256"}],
    "stateMutability": "view",
    "type": "function"
  },
  {
    "inputs": [{"internalType": "uint256", "name": "x", "type": "uint256"}],
    "name": "set",
    "outputs": [],
    "stateMutability": "nonpayable",
    "type": "function"
  }
];

// Create contract deployment transaction
const deployTx = {
  from: account,
  data: bytecode,
  gas: '0x1d4c0', // 120000
  gasPrice: '0x4a817c800',
};

const txHash = await window.ethereum.request({
  method: 'eth_sendTransaction',
  params: [deployTx],
});
```

---

## Using ethers.js

### Connect to Node

```javascript
const { ethers } = require('ethers');

// Connect to local node
const provider = new ethers.JsonRpcProvider('http://localhost:8546');

// Or with API key
const provider = new ethers.JsonRpcProvider({
  url: 'http://localhost:8546',
  headers: {
    'X-API-Key': 'your-api-key'
  }
});

// Get network info
const network = await provider.getNetwork();
console.log('Chain ID:', network.chainId);
```

### Create Wallet

```javascript
// From private key
const privateKey = '0x...';
const wallet = new ethers.Wallet(privateKey, provider);

// Or generate new
const randomWallet = ethers.Wallet.createRandom();
console.log('Address:', randomWallet.address);
console.log('Private Key:', randomWallet.privateKey);
```

### Check Balance

```javascript
const balance = await provider.getBalance('0xAddress');
console.log('Balance:', ethers.formatEther(balance), 'IDAG');
```

### Send Transaction

```javascript
const tx = await wallet.sendTransaction({
  to: '0xRecipientAddress',
  value: ethers.parseEther('1.0') // 1 IDAG
});

console.log('Transaction hash:', tx.hash);
await tx.wait(); // Wait for confirmation
console.log('Confirmed!');
```

---

## Deploying a Simple Contract

### Example: Storage Contract

**Solidity (Storage.sol):**
```solidity
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

contract Storage {
    uint256 private value;
    
    event ValueChanged(uint256 newValue);
    
    function set(uint256 _value) public {
        value = _value;
        emit ValueChanged(_value);
    }
    
    function get() public view returns (uint256) {
        return value;
    }
}
```

### Compile with solc

```bash
# Install solc
npm install -g solc

# Compile
solcjs --bin --abi Storage.sol -o build/
```

### Deploy with ethers.js

```javascript
const { ethers } = require('ethers');
const fs = require('fs');

// Load compiled contract
const bytecode = fs.readFileSync('build/Storage.bin', 'utf8');
const abi = JSON.parse(fs.readFileSync('build/Storage.abi', 'utf8'));

// Connect to provider
const provider = new ethers.JsonRpcProvider('http://localhost:8546');

// Create wallet
const privateKey = '0x...'; // Your private key
const wallet = new ethers.Wallet(privateKey, provider);

// Create contract factory
const factory = new ethers.ContractFactory(abi, bytecode, wallet);

// Deploy
console.log('Deploying contract...');
const contract = await factory.deploy();
await contract.waitForDeployment();

const address = await contract.getAddress();
console.log('Contract deployed to:', address);
```

### Interact with Contract

```javascript
// Connect to deployed contract
const contract = new ethers.Contract(address, abi, wallet);

// Call set function (transaction)
const tx = await contract.set(42);
await tx.wait();
console.log('Value set to 42');

// Call get function (read-only)
const value = await contract.get();
console.log('Stored value:', value.toString());
```

---

## Using Hardhat

### Install Hardhat

```bash
npm install --save-dev hardhat
npx hardhat init
```

### Configure hardhat.config.js

```javascript
require('@nomicfoundation/hardhat-toolbox');

module.exports = {
  solidity: '0.8.19',
  networks: {
    irondag: {
      url: 'http://localhost:8546',
      chainId: 1337,
      accounts: ['0xPrivateKey1', '0xPrivateKey2'],
      gasPrice: 20000000000, // 20 gwei
    },
  },
};
```

### Deploy Script

```javascript
// scripts/deploy.js
const hre = require('hardhat');

async function main() {
  const Storage = await hre.ethers.getContractFactory('Storage');
  const storage = await Storage.deploy();
  await storage.waitForDeployment();
  
  console.log('Storage deployed to:', await storage.getAddress());
}

main().catch(console.error);
```

### Run Deployment

```bash
npx hardhat run scripts/deploy.js --network irondag
```

---

## Using Foundry

### Install Foundry

```bash
curl -L https://foundry.paradigm.xyz | bash
foundryup
```

### Configure foundry.toml

```toml
[profile.default]
src = "src"
out = "out"
libs = ["lib"]

[rpc_endpoints]
irondag = "http://localhost:8546"
```

### Deploy

```bash
# Set private key
export PRIVATE_KEY=0x...

# Deploy
forge create src/Storage.sol:Storage \
  --rpc-url irondag \
  --private-key $PRIVATE_KEY
```

### Interact

```bash
# Call function
cast call <contract_address> "get()" --rpc-url irondag

# Send transaction
cast send <contract_address> "set(uint256)" 42 \
  --rpc-url irondag \
  --private-key $PRIVATE_KEY
```

---

## Contract Verification

Currently, IronDAG does not have an automated contract verification service. To verify:

1. **Publish source code** on the block explorer manually
2. **Provide compiler version** used (e.g., solc 0.8.19)
3. **Include optimization settings**

---

## Common Patterns

### ERC-20 Token

```solidity
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

import "@openzeppelin/contracts/token/ERC20/ERC20.sol";

contract MyToken is ERC20 {
    constructor(uint256 initialSupply) ERC20("MyToken", "MTK") {
        _mint(msg.sender, initialSupply);
    }
}
```

Deploy:
```javascript
const MyToken = await ethers.getContractFactory('MyToken');
const token = await MyToken.deploy(ethers.parseEther('1000000'));
await token.waitForDeployment();
```

### Reading Events

```javascript
// Query past events
const filter = contract.filters.ValueChanged();
const events = await contract.queryFilter(filter, -100); // Last 100 blocks

events.forEach(event => {
  console.log('New value:', event.args.newValue.toString());
});

// Listen for new events
contract.on('ValueChanged', (newValue, event) => {
  console.log('Value changed to:', newValue.toString());
});
```

### Gas Estimation

```javascript
// Estimate gas for transaction
const gasEstimate = await contract.set.estimateGas(42);
console.log('Estimated gas:', gasEstimate.toString());

// Send with custom gas limit
const tx = await contract.set(42, { gasLimit: gasEstimate * 120n / 100n });
```

---

## Troubleshooting

### Transaction Stuck Pending

**Check gas price:**
```javascript
const gasPrice = await provider.getGasPrice();
console.log('Current gas price:', gasPrice.toString());
```

**Increase gas price:**
```javascript
const tx = await contract.set(42, { gasPrice: gasPrice * 2n });
```

### Contract Deployment Fails

**Check bytecode:**
```javascript
console.log('Bytecode length:', bytecode.length);
// Should be even number, starts with 0x
```

**Increase gas limit:**
```javascript
const contract = await factory.deploy({ gasLimit: 500000 });
```

### MetaMask Connection Issues

**Reset account:**
1. MetaMask → Settings → Advanced → Reset Account

**Add network again:**
1. Remove existing IronDAG network
2. Re-add with correct Chain ID (1337)

### RPC Errors

**Authentication error:**
```javascript
const provider = new ethers.JsonRpcProvider({
  url: 'http://localhost:8546',
  headers: { 'X-API-Key': 'your-key' }
});
```

**Rate limited:**
- Wait before sending more requests
- Check node rate limit configuration

---

## Best Practices

1. **Always use EIP-155** (chain ID in transactions) for replay protection
2. **Estimate gas** before sending transactions
3. **Handle events** for async operations
4. **Test on local node** before testnet/mainnet
5. **Use try/catch** for error handling
6. **Validate inputs** in your contracts
7. **Use OpenZeppelin** libraries for standard contracts
