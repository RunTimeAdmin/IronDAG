import React, { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { SmartWallet, WalletType } from "../types";

interface AccountAbstractionProps {
  setError: (error: string | null) => void;
  setConfirmDialog: (dialog: { title: string; message: string; onConfirm: () => void } | null) => void;
}

export const AccountAbstraction: React.FC<AccountAbstractionProps> = ({ setError }) => {
  const [wallets, setWallets] = useState<SmartWallet[]>([]);
  const [selectedWallet, setSelectedWallet] = useState<string | null>(null);
  const [walletType, setWalletType] = useState<WalletType>("basic");
  const [walletOwner, setWalletOwner] = useState<string>("");
  const [multisigSigners] = useState<string[]>([]);
  const [multisigThreshold] = useState<number>(2);
  const [guardians] = useState<string[]>([]);
  const [recoveryThreshold] = useState<number>(2);
  const [spendingLimit] = useState<string>("");
  const [loading, setLoading] = useState(false);

  const createWallet = async () => {
    if (!walletOwner) {
      setError("Enter owner address");
      return;
    }
    setLoading(true);
    setError(null);
    try {
      const config: any = {};
      if (walletType === "multisig" || walletType === "combined") {
        config.multisig = {
          signers: multisigSigners,
          threshold: multisigThreshold,
        };
      }
      if (walletType === "social" || walletType === "combined") {
        config.recovery = {
          guardians: guardians,
          threshold: recoveryThreshold,
          security_delay_seconds: 86400 * 2,
        };
      }
      if (walletType === "spending" || walletType === "combined") {
        const limitBigInt = BigInt(Math.floor(parseFloat(spendingLimit || "0") * 1e18));
        config.spending_limits = {
          daily_limit: limitBigInt.toString(),
          reset_period_seconds: 86400,
        };
      }

      await invoke<any>("create_wallet", {
        walletType: walletType,
        owner: walletOwner,
        config: config,
      });

      setError(null);
      await loadWallets();
    } catch (e: any) {
      setError(e?.toString?.() ?? "Failed to create wallet");
    } finally {
      setLoading(false);
    }
  };

  const loadWallets = async () => {
    if (!walletOwner) {
      setWallets([]);
      return;
    }
    setLoading(true);
    try {
      const walletList = await invoke<SmartWallet[]>("get_owner_wallets", { owner: walletOwner });
      setWallets(Array.isArray(walletList) ? walletList : []);
    } catch (e: any) {
      setWallets([]);
    } finally {
      setLoading(false);
    }
  };

  const viewWalletDetails = async (address: string) => {
    setLoading(true);
    try {
      const wallet = await invoke<SmartWallet>("get_wallet", { address });
      alert(`Wallet Details:\n\nType: ${wallet.wallet_type}\nAddress: ${wallet.address}\nOwner: ${wallet.owner}\nNonce: ${wallet.nonce || 0}`);
    } catch (e: any) {
      setError(e?.toString?.() ?? "Failed to load wallet details");
    } finally {
      setLoading(false);
    }
  };

  return (
    <section
      style={{
        padding: "1.5rem",
        borderRadius: 16,
        background: "rgba(30, 41, 59, 0.7)",
        backdropFilter: "blur(12px)",
        border: "1px solid rgba(139, 92, 246, 0.2)",
        boxShadow: "0 8px 32px rgba(0, 0, 0, 0.3)",
      }}
    >
      <h2 style={{ fontSize: "1.4rem", marginBottom: "1.5rem", fontWeight: "600", color: "#f8fafc" }}>
        🔐 Account Abstraction
      </h2>

      {/* Wallet Creation */}
      <div style={{ marginBottom: "2rem", padding: "1.5rem", background: "rgba(139, 92, 246, 0.1)", border: "1px solid rgba(139, 92, 246, 0.3)", borderRadius: 12 }}>
        <h3 style={{ fontSize: "1.2rem", marginBottom: "1rem", fontWeight: "600", color: "#f8fafc" }}>
          Create Smart Contract Wallet
        </h3>
        <div style={{ marginBottom: "1rem" }}>
          <label style={{ display: "block", marginBottom: "0.5rem", color: "#94a3b8", fontWeight: "500" }}>
            Wallet Type
          </label>
          <select
            value={walletType}
            onChange={(e) => setWalletType(e.target.value as WalletType)}
            style={{
              width: "100%",
              padding: "0.75rem",
              borderRadius: 8,
              border: "1px solid rgba(139, 92, 246, 0.3)",
              background: "rgba(2, 6, 23, 0.6)",
              color: "#e5e7eb",
              fontSize: "0.95rem",
            }}
          >
            <option value="basic">Basic Wallet</option>
            <option value="multisig">Multi-Signature Wallet</option>
            <option value="social">Social Recovery Wallet</option>
            <option value="spending">Spending Limit Wallet</option>
            <option value="combined">Combined (Multi-Sig + Recovery + Limits)</option>
          </select>
        </div>
        <div style={{ marginBottom: "1rem" }}>
          <label style={{ display: "block", marginBottom: "0.5rem", color: "#94a3b8", fontWeight: "500" }}>
            Owner Address
          </label>
          <input
            type="text"
            value={walletOwner}
            onChange={(e) => setWalletOwner(e.target.value)}
            placeholder="0x..."
            style={{
              width: "100%",
              padding: "0.75rem",
              borderRadius: 8,
              border: "1px solid rgba(139, 92, 246, 0.3)",
              background: "rgba(2, 6, 23, 0.6)",
              color: "#e5e7eb",
              fontSize: "0.95rem",
              fontFamily: "'JetBrains Mono', monospace",
            }}
          />
        </div>
        <button
          onClick={createWallet}
          disabled={loading || !walletOwner}
          style={{
            padding: "0.75rem 2rem",
            borderRadius: 8,
            border: "none",
            background: (!walletOwner || loading) ? "rgba(139, 92, 246, 0.5)" : "linear-gradient(135deg, #8b5cf6, #7c3aed)",
            color: "white",
            cursor: (!walletOwner || loading) ? "not-allowed" : "pointer",
            fontWeight: "600",
            fontSize: "1rem",
            width: "100%",
          }}
        >
          {loading ? "⏳ Creating..." : "✨ Create Wallet"}
        </button>
      </div>

      {/* Wallet List */}
      <div>
        <h3 style={{ fontSize: "1.2rem", marginBottom: "1rem", fontWeight: "600", color: "#f8fafc" }}>
          My Smart Contract Wallets
        </h3>
        <button
          onClick={loadWallets}
          disabled={loading}
          style={{
            padding: "0.65rem 1.5rem",
            borderRadius: 8,
            border: "none",
            background: loading ? "rgba(139, 92, 246, 0.5)" : "linear-gradient(135deg, #8b5cf6, #7c3aed)",
            color: "white",
            cursor: loading ? "not-allowed" : "pointer",
            fontWeight: "600",
            fontSize: "0.95rem",
            marginBottom: "1rem",
          }}
        >
          {loading ? "⏳ Loading..." : "🔄 Refresh Wallets"}
        </button>
        {wallets.length === 0 ? (
          <div style={{ padding: "1.5rem", textAlign: "center", color: "#94a3b8", fontStyle: "italic" }}>
            No wallets found. Create your first smart contract wallet!
          </div>
        ) : (
          <div style={{ display: "grid", gap: "0.75rem" }}>
            {wallets.map((wallet) => (
              <div
                key={wallet.address}
                style={{
                  padding: "1rem",
                  background: selectedWallet === wallet.address ? "rgba(139, 92, 246, 0.15)" : "rgba(139, 92, 246, 0.05)",
                  border: selectedWallet === wallet.address ? "2px solid rgba(139, 92, 246, 0.4)" : "1px solid rgba(139, 92, 246, 0.2)",
                  borderRadius: 10,
                  cursor: "pointer",
                }}
                onClick={() => setSelectedWallet(wallet.address)}
              >
                <div style={{ display: "flex", justifyContent: "space-between", alignItems: "start" }}>
                  <div style={{ flex: 1 }}>
                    <div style={{ fontSize: "0.9rem", color: "#94a3b8", marginBottom: "0.25rem" }}>
                      {wallet.wallet_type || "Unknown"}
                    </div>
                    <div style={{
                      fontSize: "0.85rem",
                      color: "#8b5cf6",
                      fontFamily: "'JetBrains Mono', monospace",
                      wordBreak: "break-all"
                    }}>
                      {wallet.address}
                    </div>
                  </div>
                  <button
                    onClick={(e) => {
                      e.stopPropagation();
                      viewWalletDetails(wallet.address);
                    }}
                    style={{
                      padding: "0.5rem 1rem",
                      borderRadius: 8,
                      border: "none",
                      background: "linear-gradient(135deg, #06b6d4, #0891b2)",
                      color: "white",
                      cursor: "pointer",
                      fontWeight: "600",
                      fontSize: "0.85rem",
                    }}
                  >
                    View
                  </button>
                </div>
              </div>
            ))}
          </div>
        )}
      </div>
    </section>
  );
};
