# IronDAG Wallet Guide

## 📱 Wallet Overview

IronDAG wallets allow you to store, send, and receive IronDAG tokens, interact with smart contracts, and participate in staking.

---

## 🔐 Wallet Types

### **1. Desktop Wallet**
- Full node wallet
- Complete blockchain sync
- Maximum security
- Requires storage space

### **2. Mobile Wallet**
- Light client
- Fast sync
- Convenient
- Lower storage

### **3. Web Wallet**
- Browser-based
- Easy access
- Less secure
- Good for small amounts

### **4. Hardware Wallet**
- Maximum security
- Offline storage
- Best for large amounts
- Requires hardware device

---

## 🚀 Getting Started

### **Creating a Wallet**

```bash
# Install IronDAG wallet
pip install IronDAG-wallet

# Create new wallet
IronDAG-wallet create --name my_wallet

# Output:
# Wallet created: my_wallet
# Address: 0x1234...
# Backup your seed phrase!
```

### **Backing Up Your Wallet**

**CRITICAL:** Always backup your seed phrase!

```
Seed Phrase (12 words):
word1 word2 word3 ... word12

Store this securely:
- Write it down
- Store in safe place
- Never share online
- Never take screenshots
```

---

## 💰 Managing Your Wallet

### **Viewing Balance**

```bash
IronDAG-wallet balance --wallet my_wallet

# Output:
# Address: 0x1234...
# Balance: 1,000.5 IronDAG
# Pending: 0 IronDAG
```

### **Sending IronDAG**

```bash
# Send transaction
IronDAG-wallet send \
  --wallet my_wallet \
  --to 0x5678... \
  --amount 10.5 \
  --fee 0.001

# Confirm transaction
# Transaction sent: 0xabcd...
# Status: Pending
```

### **Receiving IronDAG**

```bash
# Get your address
IronDAG-wallet address --wallet my_wallet

# Share this address to receive IronDAG
# Address: 0x1234...
```

---

## 🔒 Security Best Practices

### **1. Protect Your Seed Phrase**
- ✅ Write it down physically
- ✅ Store in secure location
- ✅ Never share with anyone
- ❌ Don't store digitally
- ❌ Don't take screenshots
- ❌ Don't email it

### **2. Use Strong Passwords**
- Minimum 16 characters
- Mix of letters, numbers, symbols
- Unique password
- Use password manager

### **3. Enable 2FA**
- Two-factor authentication
- Hardware security key
- Authenticator app

### **4. Keep Software Updated**
- Update wallet regularly
- Update operating system
- Use antivirus software

---

## 📊 Transaction Management

### **Viewing Transactions**

```bash
# List all transactions
IronDAG-wallet transactions --wallet my_wallet

# View specific transaction
IronDAG-wallet tx --hash 0xabcd...
```

### **Transaction Status**

- **Pending:** Waiting for confirmation
- **Confirmed:** Included in block
- **Finalized:** GhostDAG finalized
- **Failed:** Transaction failed

### **Transaction Fees**

- **Stream A:** 0.001 IronDAG (10s blocks)
- **Stream B:** 0.001 IronDAG (1s blocks)
- **Stream C:** 0.0001 IronDAG (100ms checkpoints)

---

## 🎯 Advanced Features

### **Staking**

```bash
# Stake IronDAG
IronDAG-wallet stake \
  --wallet my_wallet \
  --amount 10000 \
  --validator validator_id

# View staking status
IronDAG-wallet staking --wallet my_wallet
```

### **Smart Contracts**

```bash
# Deploy contract
IronDAG-wallet deploy \
  --wallet my_wallet \
  --contract contract.sol

# Interact with contract
IronDAG-wallet call \
  --wallet my_wallet \
  --contract 0xabcd... \
  --function transfer \
  --args 0x5678... 100
```

### **Multi-Signature Wallets**

```bash
# Create multi-sig wallet
IronDAG-wallet create-multisig \
  --wallet my_wallet \
  --signers 0x1111... 0x2222... 0x3333... \
  --threshold 2
```

---

## 🆘 Troubleshooting

### **Wallet Not Syncing**
```bash
# Check connection
IronDAG-wallet status

# Reset sync
IronDAG-wallet reset-sync
```

### **Transaction Stuck**
```bash
# Check transaction
IronDAG-wallet tx --hash 0xabcd...

# Cancel transaction (if possible)
IronDAG-wallet cancel-tx --hash 0xabcd...
```

### **Lost Password**
- Use seed phrase to recover
- Import wallet with seed phrase
- **Cannot recover without seed phrase!**

---

## 📚 Additional Resources

- [Security Guide](SECURITY_GUIDE.md)
- [Staking Guide](STAKING_GUIDE.md)
- [Transaction Guide](TRANSACTION_GUIDE.md)
- [FAQ](FAQ.md)

---

**Status:** ✅ **Complete Wallet Guide**

