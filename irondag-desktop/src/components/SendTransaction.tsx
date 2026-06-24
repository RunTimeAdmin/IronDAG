import React, { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useKeystore } from "../hooks/useKeystore";
import type { ToastType } from "./common/Toast";

interface SendTransactionProps {
  setError: (error: string | null) => void;
  setConfirmDialog: (dialog: { title: string; message: string; onConfirm: () => void } | null) => void;
  addToast?: (message: string, type: ToastType) => void;
}

export const SendTransaction: React.FC<SendTransactionProps> = ({ setError, setConfirmDialog, addToast }) => {
  const { walletAddress, createNewKey, loadWalletAddress, loading: keystoreLoading } = useKeystore();

  const [sendTo, setSendTo] = useState<string>("");
  const [sendValue, setSendValue] = useState<string>("");
  const [sendFee, setSendFee] = useState<string>("");
  const [txHash, setTxHash] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  // Time-locked & Gasless state
  const [isTimeLocked, setIsTimeLocked] = useState<boolean>(false);
  const [executeAtBlock, setExecuteAtBlock] = useState<string>("");
  const [executeAtTimestamp, setExecuteAtTimestamp] = useState<string>("");
  const [isGasless, setIsGasless] = useState<boolean>(false);
  const [sponsorAddress, setSponsorAddress] = useState<string>("");

  const isValidAddress = (addr: string): boolean => {
    return /^(0x)?[0-9a-fA-F]{40}$/.test(addr);
  };

  const isValidAmount = (amount: string): boolean => {
    const val = parseFloat(amount);
    return !isNaN(val) && val > 0;
  };

  const handleSendTx = async () => {
    setLoading(true);
    setError(null);
    setTxHash(null);
    try {
      const valueBigInt = BigInt(Math.floor(parseFloat(sendValue) * 1e18));
      const feeBigInt = isGasless ? BigInt(0) : BigInt(Math.floor(parseFloat(sendFee) * 1e18));

      if (isTimeLocked && (executeAtBlock || executeAtTimestamp)) {
        const executeAtBlockNum = executeAtBlock ? parseInt(executeAtBlock) : undefined;
        const executeAtTimestampNum = executeAtTimestamp ? parseInt(executeAtTimestamp) : undefined;

        const hash = await invoke<string>("create_time_locked_transaction", {
          from: walletAddress,
          to: sendTo,
          value: `0x${valueBigInt.toString(16)}`,
          fee: `0x${feeBigInt.toString(16)}`,
          executeAtBlock: executeAtBlockNum,
          executeAtTimestamp: executeAtTimestampNum,
        });
        setTxHash(hash);
        addToast?.("Time-locked transaction created", "success");
      } else if (isGasless) {
        const hash = await invoke<string>("create_gasless_transaction", {
          from: walletAddress,
          to: sendTo,
          value: `0x${valueBigInt.toString(16)}`,
          fee: `0x${feeBigInt.toString(16)}`,
          sponsor: sponsorAddress,
        });
        setTxHash(hash);
        addToast?.("Gasless transaction created", "success");
      } else {
        const hash = await invoke<string>("send_transaction", {
          toAddress: sendTo,
          valueHex: `0x${valueBigInt.toString(16)}`,
          feeHex: `0x${feeBigInt.toString(16)}`,
        });
        setTxHash(hash);
        addToast?.("Transaction sent successfully", "success");
      }
    } catch (e: any) {
      setError(e?.toString?.() ?? "Failed to send transaction");
    } finally {
      setLoading(false);
    }
  };

  const sendTx = () => {
    if (!sendTo || !sendValue) {
      setError("Fill in recipient and value");
      return;
    }
    if (!isGasless && !sendFee) {
      setError("Fill in fee (or enable gasless transaction)");
      return;
    }
    if (isGasless && !sponsorAddress) {
      setError("Enter sponsor address for gasless transaction");
      return;
    }
    setConfirmDialog({
      title: "Confirm Transaction",
      message: `Send ${sendValue} IDAG to ${sendTo}?`,
      onConfirm: handleSendTx,
    });
  };

  const canSend = !loading &&
    sendTo &&
    sendValue &&
    isValidAddress(sendTo) &&
    isValidAmount(sendValue) &&
    (isGasless || (sendFee && isValidAmount(sendFee))) &&
    (!isGasless || isValidAddress(sponsorAddress));

  return (
    <section
      style={{
        padding: "1.5rem",
        borderRadius: 16,
        background: "rgba(30, 41, 59, 0.7)",
        backdropFilter: "blur(12px)",
        border: "1px solid rgba(16, 185, 129, 0.2)",
        boxShadow: "0 8px 32px rgba(0, 0, 0, 0.3)",
      }}
    >
      <h2 style={{ fontSize: "1.4rem", marginBottom: "1rem", fontWeight: "600", color: "#f8fafc" }}>
        Send Transaction
      </h2>

      {!walletAddress && (
        <div style={{
          marginBottom: "1.5rem",
          padding: "1.25rem",
          background: "rgba(251, 191, 36, 0.1)",
          border: "1px solid rgba(251, 191, 36, 0.3)",
          borderRadius: 12,
          textAlign: "center"
        }}>
          <p style={{ color: "#fbbf24", marginBottom: "1rem", fontSize: "1.05rem" }}>⚠️ No wallet loaded.</p>
          <div style={{ display: "flex", gap: "0.75rem", justifyContent: "center" }}>
            <button
              onClick={createNewKey}
              disabled={keystoreLoading}
              aria-label="Create new wallet"
              style={{
                padding: "0.65rem 1.5rem",
                borderRadius: 8,
                border: "none",
                background: keystoreLoading ? "rgba(16, 185, 129, 0.5)" : "linear-gradient(135deg, #10b981, #059669)",
                color: "white",
                cursor: keystoreLoading ? "not-allowed" : "pointer",
                fontWeight: "600",
                fontSize: "0.95rem",
              }}
            >
              {keystoreLoading ? "⏳ Creating..." : "✨ Create New Wallet"}
            </button>
            <button
              onClick={loadWalletAddress}
              disabled={keystoreLoading}
              aria-label="Load existing wallet"
              style={{
                padding: "0.65rem 1.5rem",
                borderRadius: 8,
                border: "none",
                background: keystoreLoading ? "rgba(99, 102, 241, 0.5)" : "linear-gradient(135deg, #6366f1, #4f46e5)",
                color: "white",
                cursor: keystoreLoading ? "not-allowed" : "pointer",
                fontWeight: "600",
                fontSize: "0.95rem",
              }}
            >
              {keystoreLoading ? "⏳" : "🔓"} Load Existing
            </button>
          </div>
        </div>
      )}

      {walletAddress && (
        <>
          <div style={{
            padding: "1rem",
            background: "rgba(99, 102, 241, 0.1)",
            border: "1px solid rgba(99, 102, 241, 0.2)",
            borderRadius: 10,
            marginBottom: "1.5rem",
          }}>
            <strong style={{ color: "#94a3b8", fontSize: "0.9rem" }}>Your Address</strong>
            <p style={{
              color: "#6366f1",
              fontFamily: "'JetBrains Mono', 'Courier New', monospace",
              fontSize: "0.95rem",
              marginTop: "0.5rem",
              wordBreak: "break-all",
              fontWeight: "600"
            }}>{walletAddress}</p>
          </div>

          <div style={{ marginBottom: "1rem" }}>
            <label style={{ display: "block", marginBottom: "0.5rem", color: "#94a3b8", fontWeight: "500" }}>
              To Address (0x...)
            </label>
            <input
              type="text"
              value={sendTo}
              onChange={(e) => setSendTo(e.target.value)}
              placeholder="0x..."
              aria-label="Recipient address"
              aria-invalid={sendTo ? !isValidAddress(sendTo) : undefined}
              style={{
                width: "100%",
                padding: "0.75rem",
                borderRadius: 8,
                border: sendTo && !isValidAddress(sendTo)
                  ? "1px solid rgba(239, 68, 68, 0.5)"
                  : "1px solid rgba(99, 102, 241, 0.3)",
                background: "rgba(2, 6, 23, 0.6)",
                color: "#e5e7eb",
                fontSize: "0.95rem",
                fontFamily: "'JetBrains Mono', 'Courier New', monospace",
              }}
            />
            {sendTo && !isValidAddress(sendTo) && (
              <p style={{ color: "#ef4444", fontSize: "0.85rem", marginTop: "0.25rem" }} role="alert">
                Invalid address: must be 40 hex characters (with optional 0x prefix)
              </p>
            )}
          </div>

          <div style={{ marginBottom: "1rem" }}>
            <label style={{ display: "block", marginBottom: "0.5rem", color: "#94a3b8", fontWeight: "500" }}>
              Value (IDAG)
            </label>
            <input
              type="text"
              value={sendValue}
              onChange={(e) => setSendValue(e.target.value)}
              placeholder="0.1"
              aria-label="Transaction amount"
              aria-invalid={sendValue ? !isValidAmount(sendValue) : undefined}
              style={{
                width: "100%",
                padding: "0.75rem",
                borderRadius: 8,
                border: sendValue && !isValidAmount(sendValue)
                  ? "1px solid rgba(239, 68, 68, 0.5)"
                  : "1px solid rgba(16, 185, 129, 0.3)",
                background: "rgba(2, 6, 23, 0.6)",
                color: "#e5e7eb",
                fontSize: "0.95rem",
              }}
            />
            {sendValue && !isValidAmount(sendValue) && (
              <p style={{ color: "#ef4444", fontSize: "0.85rem", marginTop: "0.25rem" }} role="alert">
                Invalid amount: must be a positive number
              </p>
            )}
          </div>

          <div style={{ marginBottom: "1.25rem" }}>
            <label style={{ display: "block", marginBottom: "0.5rem", color: "#94a3b8", fontWeight: "500" }}>
              Fee (IDAG)
            </label>
            <input
              type="text"
              value={sendFee}
              onChange={(e) => setSendFee(e.target.value)}
              placeholder="0.001"
              disabled={isGasless}
              aria-label="Transaction fee"
              style={{
                width: "100%",
                padding: "0.75rem",
                borderRadius: 8,
                border: "1px solid rgba(139, 92, 246, 0.3)",
                background: isGasless ? "rgba(2, 6, 23, 0.3)" : "rgba(2, 6, 23, 0.6)",
                color: isGasless ? "#64748b" : "#e5e7eb",
                fontSize: "0.95rem",
                opacity: isGasless ? 0.5 : 1,
              }}
            />
          </div>

          {/* Time-Locked Transaction Options */}
          <div style={{ marginBottom: "1.25rem", padding: "1rem", background: "rgba(6, 182, 212, 0.1)", border: "1px solid rgba(6, 182, 212, 0.3)", borderRadius: 10 }}>
            <label style={{ display: "flex", alignItems: "center", cursor: "pointer", color: "#94a3b8", fontWeight: "500" }}>
              <input
                type="checkbox"
                checked={isTimeLocked}
                onChange={(e) => setIsTimeLocked(e.target.checked)}
                aria-label="Enable time-locked transaction"
                style={{ marginRight: "0.5rem", width: "18px", height: "18px", cursor: "pointer" }}
              />
              <span>⏰ Time-Locked Transaction</span>
            </label>
            {isTimeLocked && (
              <div style={{ marginTop: "1rem", display: "flex", flexDirection: "column", gap: "0.75rem" }}>
                <div>
                  <label style={{ display: "block", marginBottom: "0.5rem", color: "#94a3b8", fontSize: "0.9rem" }}>
                    Execute at Block Number (optional)
                  </label>
                  <input
                    type="number"
                    value={executeAtBlock}
                    onChange={(e) => setExecuteAtBlock(e.target.value)}
                    placeholder="e.g., 1000"
                    aria-label="Execute at block number"
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
                <div>
                  <label style={{ display: "block", marginBottom: "0.5rem", color: "#94a3b8", fontSize: "0.9rem" }}>
                    OR Execute at Timestamp (Unix timestamp, optional)
                  </label>
                  <input
                    type="number"
                    value={executeAtTimestamp}
                    onChange={(e) => setExecuteAtTimestamp(e.target.value)}
                    placeholder="e.g., 1704067200"
                    aria-label="Execute at unix timestamp"
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
              </div>
            )}
          </div>

          {/* Gasless Transaction Options */}
          <div style={{ marginBottom: "1.25rem", padding: "1rem", background: "rgba(16, 185, 129, 0.1)", border: "1px solid rgba(16, 185, 129, 0.3)", borderRadius: 10 }}>
            <label style={{ display: "flex", alignItems: "center", cursor: "pointer", color: "#94a3b8", fontWeight: "500" }}>
              <input
                type="checkbox"
                checked={isGasless}
                onChange={(e) => setIsGasless(e.target.checked)}
                aria-label="Enable gasless transaction"
                style={{ marginRight: "0.5rem", width: "18px", height: "18px", cursor: "pointer" }}
              />
              <span>🎁 Gasless Transaction (Sponsored)</span>
            </label>
            {isGasless && (
              <div style={{ marginTop: "1rem" }}>
                <label style={{ display: "block", marginBottom: "0.5rem", color: "#94a3b8", fontSize: "0.9rem" }}>
                  Sponsor Address (who pays the fee)
                </label>
                <input
                  type="text"
                  value={sponsorAddress}
                  onChange={(e) => setSponsorAddress(e.target.value)}
                  placeholder="0x..."
                  aria-label="Sponsor address"
                  style={{
                    width: "100%",
                    padding: "0.65rem",
                    borderRadius: 8,
                    border: "1px solid rgba(16, 185, 129, 0.3)",
                    background: "rgba(2, 6, 23, 0.6)",
                    color: "#e5e7eb",
                    fontSize: "0.9rem",
                    fontFamily: "'JetBrains Mono', monospace",
                  }}
                />
              </div>
            )}
          </div>

          <button
            onClick={sendTx}
            disabled={!canSend}
            aria-label="Send transaction"
            style={{
              padding: "0.75rem 2rem",
              borderRadius: 8,
              border: "none",
              background: !canSend
                ? "rgba(16, 185, 129, 0.5)"
                : "linear-gradient(135deg, #10b981, #059669)",
              color: "white",
              cursor: !canSend ? "not-allowed" : "pointer",
              fontWeight: "600",
              fontSize: "1rem",
              boxShadow: !canSend ? "none" : "0 4px 12px rgba(16, 185, 129, 0.4)",
              transition: "all 0.3s ease",
              opacity: !canSend ? 0.6 : 1,
              width: "100%",
            }}
          >
            {loading ? "⏳ Sending..." : "🚀 Send Transaction"}
          </button>

          {txHash && (
            <div
              style={{
                marginTop: "1.5rem",
                padding: "1.25rem",
                borderRadius: 12,
                background: "rgba(16, 185, 129, 0.1)",
                border: "1px solid rgba(16, 185, 129, 0.3)",
              }}
            >
              <strong style={{ color: "#10b981", fontSize: "1.1rem" }}>✅ Transaction Sent!</strong>
              <p style={{ color: "#94a3b8", fontSize: "0.9rem", marginTop: "0.5rem", marginBottom: "0.5rem" }}>Transaction Hash</p>
              <p style={{
                color: "#06b6d4",
                fontFamily: "'JetBrains Mono', 'Courier New', monospace",
                fontSize: "0.9rem",
                wordBreak: "break-all",
                background: "rgba(2, 6, 23, 0.8)",
                padding: "0.75rem",
                borderRadius: 8,
              }}>{txHash}</p>
            </div>
          )}
        </>
      )}
    </section>
  );
};
