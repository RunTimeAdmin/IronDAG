import React, { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { Contact, Account, Reputation, ReputationFactors } from "../types";
import type { ToastType } from "./common/Toast";

interface WalletProps {
  setError: (error: string | null) => void;
  setConfirmDialog: (dialog: { title: string; message: string; onConfirm: () => void } | null) => void;
  addToast?: (message: string, type: ToastType) => void;
}

export const Wallet: React.FC<WalletProps> = ({ setError, setConfirmDialog, addToast: _addToast }) => {
  const [walletAddress, setWalletAddress] = useState<string>("");
  const [walletBalanceHex, setWalletBalanceHex] = useState<string | null>(null);
  const [walletNonceHex, setWalletNonceHex] = useState<string | null>(null);
  const [reputation, setReputation] = useState<Reputation | null>(null);
  const [reputationFactors, setReputationFactors] = useState<ReputationFactors | null>(null);
  const [loading, setLoading] = useState(false);

  // Address Book state
  const [contacts, setContacts] = useState<Contact[]>([]);
  const [showAddContact, setShowAddContact] = useState(false);
  const [contactName, setContactName] = useState("");
  const [contactAddress, setContactAddress] = useState("");
  const [contactNotes, setContactNotes] = useState("");

  // Multi-Account state
  const [accounts, setAccounts] = useState<Account[]>([]);
  const [showAddAccount, setShowAddAccount] = useState(false);
  const [accountName, setAccountName] = useState("");
  const [accountAddress, setAccountAddress] = useState("");
  const [selectedAccount, setSelectedAccount] = useState<string | null>(null);

  useEffect(() => {
    loadContacts();
    loadAccounts();
  }, []);

  const loadWallet = async () => {
    if (!walletAddress) {
      setError("Please enter an address.");
      return;
    }
    setLoading(true);
    setError(null);
    try {
      const balanceHex = await invoke<string>("get_balance", { address: walletAddress });
      const nonceHex = await invoke<string>("get_nonce", { address: walletAddress });
      setWalletBalanceHex(balanceHex);
      setWalletNonceHex(nonceHex);

      // Load reputation
      try {
        const rep = await invoke<Reputation>("get_reputation", { address: walletAddress });
        const factors = await invoke<ReputationFactors>("get_reputation_factors", { address: walletAddress });
        if (rep) setReputation(rep);
        if (factors) setReputationFactors(factors);
      } catch (e) {
        // Reputation might not be available, ignore
      }
    } catch (e: any) {
      setError(e?.toString?.() ?? "Failed to load wallet");
    } finally {
      setLoading(false);
    }
  };

  const formatBalance = (balanceHex: string | null): { raw: string; mshw: string } => {
    if (!balanceHex) return { raw: "-", mshw: "-" };
    try {
      const v = BigInt(balanceHex);
      const denom = 10n ** 18n;
      const whole = v / denom;
      const frac = v % denom;
      const fracStr = (frac / (10n ** 12n)).toString().padStart(6, "0");
      return { raw: balanceHex, mshw: `${whole.toString()}.${fracStr}` };
    } catch {
      return { raw: balanceHex, mshw: "?" };
    }
  };

  const loadContacts = async () => {
    try {
      const result = await invoke<Contact[]>("get_contacts");
      setContacts(result);
    } catch (e: any) {
      setError(e?.toString?.() ?? "Failed to load contacts");
    }
  };

  const addContact = async () => {
    if (!contactName || !contactAddress) {
      setError("Name and address required");
      return;
    }
    setLoading(true);
    try {
      await invoke("add_contact", {
        name: contactName,
        address: contactAddress,
        notes: contactNotes || null,
      });
      setContactName("");
      setContactAddress("");
      setContactNotes("");
      setShowAddContact(false);
      await loadContacts();
    } catch (e: any) {
      setError(e?.toString?.() ?? "Failed to add contact");
    } finally {
      setLoading(false);
    }
  };

  const removeContact = async (address: string) => {
    const contact = contacts.find((c) => c.address === address);
    const name = contact?.name || address.slice(0, 10) + "...";
    setConfirmDialog({
      title: "Remove Contact",
      message: `Remove ${name} from your address book?`,
      onConfirm: async () => {
        setLoading(true);
        try {
          await invoke("remove_contact", { address });
          await loadContacts();
        } catch (e: any) {
          setError(e?.toString?.() ?? "Failed to remove contact");
        } finally {
          setLoading(false);
        }
      }
    });
  };

  const loadAccounts = async () => {
    try {
      const result = await invoke<Account[]>("get_accounts");
      setAccounts(result);
      if (result.length > 0 && !selectedAccount) {
        setSelectedAccount(result[0].address);
      }
    } catch (e: any) {
      setError(e?.toString?.() ?? "Failed to load accounts");
    }
  };

  const addAccount = async () => {
    if (!accountName || !accountAddress) {
      setError("Name and address required");
      return;
    }
    setLoading(true);
    try {
      await invoke("add_account", {
        name: accountName,
        address: accountAddress,
      });
      setAccountName("");
      setAccountAddress("");
      setShowAddAccount(false);
      await loadAccounts();
    } catch (e: any) {
      setError(e?.toString?.() ?? "Failed to add account");
    } finally {
      setLoading(false);
    }
  };

  const removeAccount = async (address: string) => {
    const account = accounts.find((a) => a.address === address);
    const name = account?.name || address.slice(0, 10) + "...";
    setConfirmDialog({
      title: "Remove Account",
      message: `Remove ${name} from your accounts?`,
      onConfirm: async () => {
        setLoading(true);
        try {
          await invoke("remove_account", { address });
          await loadAccounts();
          if (selectedAccount === address && accounts.length > 0) {
            setSelectedAccount(accounts[0].address);
          }
        } catch (e: any) {
          setError(e?.toString?.() ?? "Failed to remove account");
        } finally {
          setLoading(false);
        }
      }
    });
  };

  const { raw, mshw } = formatBalance(walletBalanceHex);

  return (
    <>
      {/* Wallet Inspector */}
      <section
        style={{
          padding: "1.5rem",
          borderRadius: 16,
          background: "rgba(30, 41, 59, 0.7)",
          backdropFilter: "blur(12px)",
          border: "1px solid rgba(99, 102, 241, 0.2)",
          boxShadow: "0 8px 32px rgba(0, 0, 0, 0.3)",
        }}
      >
        <h2 style={{ fontSize: "1.4rem", marginBottom: "1rem", fontWeight: "600", color: "#f8fafc" }}>
          Wallet Inspector
        </h2>
        <div style={{ marginBottom: "1rem" }}>
          <label style={{ display: "block", marginBottom: "0.5rem", color: "#94a3b8", fontWeight: "500" }}>
            Address (0x…)
          </label>
          <input
            type="text"
            value={walletAddress}
            onChange={(e) => setWalletAddress(e.target.value)}
            placeholder="0x..."
            aria-label="Wallet address"
            style={{
              width: "100%",
              padding: "0.75rem",
              borderRadius: 8,
              border: "1px solid rgba(99, 102, 241, 0.3)",
              background: "rgba(2, 6, 23, 0.6)",
              color: "#e5e7eb",
              fontSize: "0.95rem",
              fontFamily: "'JetBrains Mono', 'Courier New', monospace",
            }}
          />
        </div>
        <button
          onClick={loadWallet}
          disabled={loading || !walletAddress}
          aria-label="Load wallet information"
          style={{
            padding: "0.65rem 1.5rem",
            borderRadius: 8,
            border: "none",
            background: (!walletAddress || loading) ? "rgba(99, 102, 241, 0.5)" : "linear-gradient(135deg, #6366f1, #4f46e5)",
            color: "white",
            cursor: (!walletAddress || loading) ? "not-allowed" : "pointer",
            marginBottom: "1.25rem",
            fontWeight: "600",
            fontSize: "0.95rem",
          }}
        >
          {loading ? "⏳ Loading..." : "🔍 Load Wallet"}
        </button>

        {walletBalanceHex && walletNonceHex && (
          <div
            style={{
              marginTop: "0.5rem",
              padding: "1.25rem",
              borderRadius: 12,
              background: "rgba(6, 182, 212, 0.1)",
              border: "1px solid rgba(6, 182, 212, 0.3)",
              backdropFilter: "blur(8px)",
            }}
          >
            <div style={{ marginBottom: "0.75rem" }}>
              <strong style={{ color: "#94a3b8", fontSize: "0.9rem" }}>Balance (raw)</strong>
              <p style={{
                color: "#06b6d4",
                fontFamily: "'JetBrains Mono', 'Courier New', monospace",
                fontSize: "0.95rem",
                marginTop: "0.25rem",
                wordBreak: "break-all"
              }}>{raw}</p>
            </div>
            <div style={{ marginBottom: "0.75rem" }}>
              <strong style={{ color: "#94a3b8", fontSize: "0.9rem" }}>Balance (IDAG)</strong>
              <p style={{
                color: "#10b981",
                fontSize: "1.5rem",
                fontWeight: "700",
                marginTop: "0.25rem"
              }}>💰 {mshw}</p>
            </div>
            <div>
              <strong style={{ color: "#94a3b8", fontSize: "0.9rem" }}>Nonce</strong>
              <p style={{
                color: "#8b5cf6",
                fontFamily: "'JetBrains Mono', 'Courier New', monospace",
                fontSize: "1.1rem",
                fontWeight: "600",
                marginTop: "0.25rem"
              }}>{walletNonceHex}</p>
            </div>
          </div>
        )}

        {/* Reputation Display */}
        {walletAddress && reputation && (
          <div
            style={{
              marginTop: "1.5rem",
              padding: "1.25rem",
              borderRadius: 12,
              background: "rgba(16, 185, 129, 0.1)",
              border: "1px solid rgba(16, 185, 129, 0.3)",
              backdropFilter: "blur(8px)",
            }}
          >
            <h3 style={{ fontSize: "1.1rem", marginBottom: "1rem", fontWeight: "600", color: "#f8fafc" }}>
              ⭐ Reputation
            </h3>
            <div style={{ marginBottom: "0.75rem" }}>
              <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: "0.5rem" }}>
                <strong style={{ color: "#94a3b8", fontSize: "0.9rem" }}>Score</strong>
                <span style={{
                  color: reputation.score >= 80 ? "#10b981" : reputation.score >= 40 ? "#fbbf24" : "#ef4444",
                  fontSize: "1.5rem",
                  fontWeight: "700"
                }}>
                  {reputation.score}/100
                </span>
              </div>
              <div style={{
                color: reputation.level === "High" ? "#10b981" : reputation.level === "Medium" ? "#fbbf24" : "#ef4444",
                fontSize: "0.95rem",
                fontWeight: "600"
              }}>
                Level: {reputation.level}
              </div>
            </div>
            {reputationFactors && (
              <div style={{
                marginTop: "1rem",
                padding: "1rem",
                background: "rgba(2, 6, 23, 0.6)",
                borderRadius: 8,
                fontSize: "0.85rem"
              }}>
                <div style={{ marginBottom: "0.5rem", color: "#94a3b8" }}>Factors:</div>
                <div style={{ display: "grid", gridTemplateColumns: "repeat(2, 1fr)", gap: "0.5rem" }}>
                  <div>✅ Successful: {reputationFactors.successful_txs || 0}</div>
                  <div>❌ Failed: {reputationFactors.failed_txs || 0}</div>
                  <div>⛏️ Blocks: {reputationFactors.blocks_mined || 0}</div>
                  <div>📅 Age: {reputationFactors.account_age_days || 0} days</div>
                  <div>💰 Value: {((reputationFactors.total_value_transacted || 0) / 1e18).toFixed(2)} IDAG</div>
                  <div>👥 Contacts: {reputationFactors.unique_contacts || 0}</div>
                </div>
              </div>
            )}
          </div>
        )}
      </section>

      {/* Address Book */}
      <section
        style={{
          marginTop: "1.5rem",
          padding: "1.5rem",
          borderRadius: 16,
          background: "rgba(30, 41, 59, 0.7)",
          backdropFilter: "blur(12px)",
          border: "1px solid rgba(236, 72, 153, 0.2)",
          boxShadow: "0 8px 32px rgba(0, 0, 0, 0.3)",
        }}
      >
        <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: "1rem" }}>
          <h2 style={{ fontSize: "1.4rem", fontWeight: "600", color: "#f8fafc" }}>
            📖 Address Book
          </h2>
          <button
            onClick={() => setShowAddContact(!showAddContact)}
            aria-label={showAddContact ? "Cancel adding contact" : "Add new contact"}
            style={{
              padding: "0.5rem 1rem",
              borderRadius: 8,
              border: "none",
              background: "linear-gradient(135deg, #ec4899, #db2777)",
              color: "white",
              cursor: "pointer",
              fontWeight: "600",
              fontSize: "0.9rem",
            }}
          >
            {showAddContact ? "❌ Cancel" : "➕ Add Contact"}
          </button>
        </div>

        {showAddContact && (
          <div style={{
            marginBottom: "1.5rem",
            padding: "1rem",
            background: "rgba(236, 72, 153, 0.1)",
            border: "1px solid rgba(236, 72, 153, 0.3)",
            borderRadius: 10,
          }}>
            <div style={{ marginBottom: "0.75rem" }}>
              <label style={{ display: "block", marginBottom: "0.5rem", color: "#94a3b8", fontWeight: "500", fontSize: "0.9rem" }}>
                Name
              </label>
              <input
                type="text"
                value={contactName}
                onChange={(e) => setContactName(e.target.value)}
                placeholder="Alice"
                aria-label="Contact name"
                style={{
                  width: "100%",
                  padding: "0.65rem",
                  borderRadius: 8,
                  border: "1px solid rgba(236, 72, 153, 0.3)",
                  background: "rgba(2, 6, 23, 0.6)",
                  color: "#e5e7eb",
                  fontSize: "0.9rem",
                }}
              />
            </div>
            <div style={{ marginBottom: "0.75rem" }}>
              <label style={{ display: "block", marginBottom: "0.5rem", color: "#94a3b8", fontWeight: "500", fontSize: "0.9rem" }}>
                Address
              </label>
              <input
                type="text"
                value={contactAddress}
                onChange={(e) => setContactAddress(e.target.value)}
                placeholder="0x..."
                aria-label="Contact address"
                style={{
                  width: "100%",
                  padding: "0.65rem",
                  borderRadius: 8,
                  border: "1px solid rgba(236, 72, 153, 0.3)",
                  background: "rgba(2, 6, 23, 0.6)",
                  color: "#e5e7eb",
                  fontSize: "0.9rem",
                  fontFamily: "'JetBrains Mono', monospace",
                }}
              />
            </div>
            <div style={{ marginBottom: "1rem" }}>
              <label style={{ display: "block", marginBottom: "0.5rem", color: "#94a3b8", fontWeight: "500", fontSize: "0.9rem" }}>
                Notes (optional)
              </label>
              <input
                type="text"
                value={contactNotes}
                onChange={(e) => setContactNotes(e.target.value)}
                placeholder="Friend, exchange, etc."
                aria-label="Contact notes optional"
                style={{
                  width: "100%",
                  padding: "0.65rem",
                  borderRadius: 8,
                  border: "1px solid rgba(236, 72, 153, 0.3)",
                  background: "rgba(2, 6, 23, 0.6)",
                  color: "#e5e7eb",
                  fontSize: "0.9rem",
                }}
              />
            </div>
            <button
              onClick={addContact}
              disabled={loading}
              aria-label="Save contact"
              style={{
                padding: "0.65rem 1.5rem",
                borderRadius: 8,
                border: "none",
                background: loading ? "rgba(236, 72, 153, 0.5)" : "linear-gradient(135deg, #ec4899, #db2777)",
                color: "white",
                cursor: loading ? "not-allowed" : "pointer",
                fontWeight: "600",
                fontSize: "0.95rem",
                width: "100%",
              }}
            >
              {loading ? "⏳ Saving..." : "💾 Save Contact"}
            </button>
          </div>
        )}

        {contacts.length === 0 ? (
          <div style={{ padding: "1.5rem", textAlign: "center", color: "#94a3b8", fontStyle: "italic" }}>
            No contacts yet. Add your first contact!
          </div>
        ) : (
          <div style={{ display: "grid", gap: "0.75rem" }}>
            {contacts.map((contact) => (
              <div
                key={contact.address}
                style={{
                  padding: "1rem",
                  background: "rgba(236, 72, 153, 0.1)",
                  border: "1px solid rgba(236, 72, 153, 0.2)",
                  borderRadius: 10,
                  display: "flex",
                  justifyContent: "space-between",
                  alignItems: "center",
                }}
              >
                <div style={{ flex: 1 }}>
                  <div style={{ fontSize: "1.1rem", fontWeight: "600", color: "#ec4899", marginBottom: "0.25rem" }}>
                    {contact.name}
                  </div>
                  <div style={{
                    fontSize: "0.85rem",
                    color: "#06b6d4",
                    fontFamily: "'JetBrains Mono', monospace",
                    wordBreak: "break-all",
                    marginBottom: "0.25rem"
                  }}>
                    {contact.address}
                  </div>
                  {contact.notes && (
                    <div style={{ fontSize: "0.8rem", color: "#94a3b8", fontStyle: "italic" }}>
                      {contact.notes}
                    </div>
                  )}
                </div>
                <button
                  onClick={() => removeContact(contact.address)}
                  disabled={loading}
                  aria-label={`Remove contact ${contact.name}`}
                  style={{
                    padding: "0.5rem 1rem",
                    borderRadius: 8,
                    border: "none",
                    background: "linear-gradient(135deg, #ef4444, #dc2626)",
                    color: "white",
                    cursor: loading ? "not-allowed" : "pointer",
                    fontWeight: "600",
                    fontSize: "0.85rem",
                  }}
                >
                  🗑️
                </button>
              </div>
            ))}
          </div>
        )}
      </section>

      {/* Multi-Account */}
      <section
        style={{
          marginTop: "1.5rem",
          padding: "1.5rem",
          borderRadius: 16,
          background: "rgba(30, 41, 59, 0.7)",
          backdropFilter: "blur(12px)",
          border: "1px solid rgba(6, 182, 212, 0.2)",
          boxShadow: "0 8px 32px rgba(0, 0, 0, 0.3)",
        }}
      >
        <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: "1rem" }}>
          <h2 style={{ fontSize: "1.4rem", fontWeight: "600", color: "#f8fafc" }}>
            💼 My Accounts
          </h2>
          <button
            onClick={() => setShowAddAccount(!showAddAccount)}
            aria-label={showAddAccount ? "Cancel adding account" : "Add new account"}
            style={{
              padding: "0.5rem 1rem",
              borderRadius: 8,
              border: "none",
              background: "linear-gradient(135deg, #06b6d4, #0891b2)",
              color: "white",
              cursor: "pointer",
              fontWeight: "600",
              fontSize: "0.9rem",
            }}
          >
            {showAddAccount ? "❌ Cancel" : "➕ Add Account"}
          </button>
        </div>

        {showAddAccount && (
          <div style={{
            marginBottom: "1.5rem",
            padding: "1rem",
            background: "rgba(6, 182, 212, 0.1)",
            border: "1px solid rgba(6, 182, 212, 0.3)",
            borderRadius: 10,
          }}>
            <div style={{ marginBottom: "0.75rem" }}>
              <label style={{ display: "block", marginBottom: "0.5rem", color: "#94a3b8", fontWeight: "500", fontSize: "0.9rem" }}>
                Account Name
              </label>
              <input
                type="text"
                value={accountName}
                onChange={(e) => setAccountName(e.target.value)}
                placeholder="Main Account"
                aria-label="Account name"
                style={{
                  width: "100%",
                  padding: "0.65rem",
                  borderRadius: 8,
                  border: "1px solid rgba(6, 182, 212, 0.3)",
                  background: "rgba(2, 6, 23, 0.6)",
                  color: "#e5e7eb",
                  fontSize: "0.9rem",
                }}
              />
            </div>
            <div style={{ marginBottom: "1rem" }}>
              <label style={{ display: "block", marginBottom: "0.5rem", color: "#94a3b8", fontWeight: "500", fontSize: "0.9rem" }}>
                Address
              </label>
              <input
                type="text"
                value={accountAddress}
                onChange={(e) => setAccountAddress(e.target.value)}
                placeholder="0x..."
                aria-label="Account address"
                style={{
                  width: "100%",
                  padding: "0.65rem",
                  borderRadius: 8,
                  border: "1px solid rgba(6, 182, 212, 0.3)",
                  background: "rgba(2, 6, 23, 0.6)",
                  color: "#e5e7eb",
                  fontSize: "0.9rem",
                  fontFamily: "'JetBrains Mono', monospace",
                }}
              />
            </div>
            <button
              onClick={addAccount}
              disabled={loading}
              aria-label="Save account"
              style={{
                padding: "0.65rem 1.5rem",
                borderRadius: 8,
                border: "none",
                background: loading ? "rgba(6, 182, 212, 0.5)" : "linear-gradient(135deg, #06b6d4, #0891b2)",
                color: "white",
                cursor: loading ? "not-allowed" : "pointer",
                fontWeight: "600",
                fontSize: "0.95rem",
                width: "100%",
              }}
            >
              {loading ? "⏳ Saving..." : "💾 Save Account"}
            </button>
          </div>
        )}

        {accounts.length === 0 ? (
          <div style={{ padding: "1.5rem", textAlign: "center", color: "#94a3b8", fontStyle: "italic" }}>
            No accounts added. Create or add your first account!
          </div>
        ) : (
          <div style={{ display: "grid", gap: "0.75rem" }}>
            {accounts.map((account) => (
              <div
                key={account.address}
                style={{
                  padding: "1rem",
                  background: selectedAccount === account.address
                    ? "rgba(6, 182, 212, 0.15)"
                    : "rgba(6, 182, 212, 0.05)",
                  border: selectedAccount === account.address
                    ? "2px solid rgba(6, 182, 212, 0.4)"
                    : "1px solid rgba(6, 182, 212, 0.2)",
                  borderRadius: 10,
                  display: "flex",
                  justifyContent: "space-between",
                  alignItems: "center",
                  cursor: "pointer",
                }}
                onClick={() => setSelectedAccount(account.address)}
              >
                <div style={{ flex: 1 }}>
                  <div style={{
                    fontSize: "1.1rem",
                    fontWeight: "600",
                    color: selectedAccount === account.address ? "#06b6d4" : "#64748b",
                    marginBottom: "0.25rem"
                  }}>
                    {selectedAccount === account.address && "✔️ "}{account.name}
                  </div>
                  <div style={{
                    fontSize: "0.85rem",
                    color: "#06b6d4",
                    fontFamily: "'JetBrains Mono', monospace",
                    wordBreak: "break-all"
                  }}>
                    {account.address}
                  </div>
                </div>
                <button
                  onClick={(e) => {
                    e.stopPropagation();
                    removeAccount(account.address);
                  }}
                  disabled={loading}
                  aria-label={`Remove account ${account.name}`}
                  style={{
                    padding: "0.5rem 1rem",
                    borderRadius: 8,
                    border: "none",
                    background: "linear-gradient(135deg, #ef4444, #dc2626)",
                    color: "white",
                    cursor: loading ? "not-allowed" : "pointer",
                    fontWeight: "600",
                    fontSize: "0.85rem",
                  }}
                >
                  🗑️
                </button>
              </div>
            ))}
          </div>
        )}
      </section>
    </>
  );
};
