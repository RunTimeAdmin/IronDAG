# IronDAG Staking Guide

## 🎯 What is Staking?

Staking allows you to lock IronDAG tokens to participate in network validation and earn rewards.

---

## 💡 How Staking Works

### **Validator Staking**
- Lock IronDAG tokens
- Participate in GhostDAG consensus
- Validate blocks
- Earn staking rewards
- Risk: Slashing for misbehavior

### **Delegator Staking**
- Delegate IronDAG to validators
- Earn rewards (minus validator fee)
- Lower risk (no slashing)
- Can unstake anytime

---

## 🚀 Getting Started

### **Requirements**

```
Minimum Stake: 100,000 IronDAG
Hardware: Validator node requirements
Uptime: 99%+ required
Stake Period: Minimum 30 days
```

### **Become a Validator**

```bash
# 1. Set up validator node
IronDAG-validator setup

# 2. Stake IronDAG
IronDAG-validator stake --amount 100000

# 3. Start validator
IronDAG-validator start

# 4. Monitor status
IronDAG-validator status
```

### **Delegate to Validator**

```bash
# 1. List validators
IronDAG-wallet validators

# 2. Delegate to validator
IronDAG-wallet delegate \
  --validator validator_id \
  --amount 10000

# 3. View delegation
IronDAG-wallet delegations
```

---

## 💰 Staking Rewards

### **Reward Calculation**

```
Annual Reward Rate: 5-10% (variable)
Based on:
- Network participation
- Validator performance
- Total staked amount
- Network fees
```

### **Reward Distribution**

```
Validator Rewards:
- Block rewards: 50%
- Transaction fees: 30%
- Staking rewards: 20%

Delegator Rewards:
- Validator rewards - validator fee (5-10%)
```

### **Example**

```
Stake: 100,000 IronDAG
Annual Rate: 8%
Annual Reward: 8,000 IronDAG
Monthly Reward: ~667 IronDAG
```

---

## ⚠️ Risks & Slashing

### **Slashing Conditions**

```
1. Double Signing
   - Penalty: 100% of stake
   - Automatic slashing

2. Downtime
   - Penalty: 1% per hour
   - Max: 5% per day

3. Invalid Blocks
   - Penalty: 10% of stake
   - Automatic slashing
```

### **Risk Mitigation**

```
✅ Run reliable infrastructure
✅ Monitor validator status
✅ Use backup systems
✅ Keep software updated
✅ Follow best practices
```

---

## 📊 Staking Dashboard

### **View Staking Status**

```bash
# Validator status
IronDAG-validator status

# Output:
# Validator ID: validator_123
# Staked: 100,000 IronDAG
# Status: Active
# Uptime: 99.5%
# Rewards: 8,000 IronDAG/year
# Slashing Risk: Low
```

### **View Rewards**

```bash
# View staking rewards
IronDAG-wallet staking-rewards

# Output:
# Total Staked: 100,000 IronDAG
# Rewards Earned: 667 IronDAG
# Pending Rewards: 50 IronDAG
# Annual Rate: 8%
```

---

## 🔄 Unstaking

### **Unstake Process**

```bash
# 1. Request unstake
IronDAG-validator unstake --amount 50000

# 2. Wait for unlock period (30 days)
# 3. Withdraw unlocked tokens
IronDAG-validator withdraw
```

### **Unlock Period**

```
Unstake Request → 30 Day Lock → Withdraw Available
```

---

## 📚 Best Practices

### **For Validators**
- ✅ Maintain 99%+ uptime
- ✅ Monitor infrastructure
- ✅ Keep software updated
- ✅ Use backup systems
- ✅ Follow security practices

### **For Delegators**
- ✅ Research validators
- ✅ Diversify delegations
- ✅ Monitor validator performance
- ✅ Check validator fees
- ✅ Review slashing history

---

## 🆘 Troubleshooting

### **Validator Offline**
```bash
# Check status
IronDAG-validator status

# Restart validator
IronDAG-validator restart

# Check logs
IronDAG-validator logs
```

### **Low Rewards**
- Check validator performance
- Review network conditions
- Verify stake amount
- Check validator fee

---

## 📖 Additional Resources

- [Validator Setup Guide](VALIDATOR_SETUP.md)
- [Security Guide](SECURITY_GUIDE.md)
- [FAQ](FAQ.md)

---

**Status:** ✅ **Complete Staking Guide**

