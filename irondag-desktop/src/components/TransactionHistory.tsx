import React, { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useKeystore } from "../hooks/useKeystore";
import type { Transaction } from "../types";

interface TransactionHistoryProps {
  setError: (error: string | null) => void;
}

export const TransactionHistory: React.FC<TransactionHistoryProps> = ({ setError }) => {
  const { walletAddress } = useKeystore();
  const [txHistory, setTxHistory] = useState<Transaction[]>([]);
  const [loading, setLoading] = useState(false);
  const [exporting, setExporting] = useState(false);
  const txHistoryLimit = 50;

  const exportTransactions = async (format: 'csv' | 'json') => {
    if (!txHistory.length) return;
    
    setExporting(true);
    
    try {
      let content: string;
      let filename: string;
      
      if (format === 'csv') {
        const headers = 'Hash,From,To,Value,BlockNumber,Timestamp,Direction\n';
        const rows = txHistory.map(tx => {
          const timestamp = parseInt(tx.timestamp, 16);
          const date = new Date(timestamp * 1000).toISOString();
          const valueIDAG = (Number(BigInt(tx.value)) / 1e18).toFixed(6);
          return `${tx.hash},${tx.from},${tx.to},${valueIDAG},${parseInt(tx.block_number, 16)},${date},${tx.direction || ''}`;
        }).join('\n');
        content = headers + rows;
        filename = `irondag-transactions-${Date.now()}.csv`;
      } else {
        // Format transactions for JSON export with human-readable values
        const formattedTxs = txHistory.map(tx => {
          const timestamp = parseInt(tx.timestamp, 16);
          return {
            ...tx,
            value_idag: (Number(BigInt(tx.value)) / 1e18).toFixed(6),
            block_number_dec: parseInt(tx.block_number, 16),
            timestamp_iso: new Date(timestamp * 1000).toISOString(),
          };
        });
        content = JSON.stringify(formattedTxs, null, 2);
        filename = `irondag-transactions-${Date.now()}.json`;
      }
      
      // Use Tauri save dialog
      try {
        const { save } = await import('@tauri-apps/plugin-dialog');
        const { writeTextFile } = await import('@tauri-apps/plugin-fs');
        
        const filePath = await save({
          defaultPath: filename,
          filters: format === 'csv' 
            ? [{ name: 'CSV', extensions: ['csv'] }]
            : [{ name: 'JSON', extensions: ['json'] }]
        });
        
        if (filePath) {
          await writeTextFile(filePath, content);
          // Success - could add toast notification here
        }
      } catch (e) {
        console.error('Tauri export failed, falling back to browser download:', e);
        // Fallback: use browser download
        const blob = new Blob([content], { type: format === 'csv' ? 'text/csv' : 'application/json' });
        const url = URL.createObjectURL(blob);
        const a = document.createElement('a');
        a.href = url;
        a.download = filename;
        document.body.appendChild(a);
        a.click();
        document.body.removeChild(a);
        URL.revokeObjectURL(url);
      }
    } catch (e) {
      console.error('Export failed:', e);
      setError(`Export failed: ${e}`);
    } finally {
      setExporting(false);
    }
  };

  const loadTxHistory = async () => {
    if (!walletAddress) {
      setError("No wallet loaded");
      return;
    }
    setLoading(true);
    setError(null);
    try {
      const result = await invoke<{ transactions: Transaction[] }>("get_address_transactions", {
        address: walletAddress,
        limit: txHistoryLimit,
      });
      setTxHistory(result.transactions || []);
    } catch (e: any) {
      setError(e?.toString?.() ?? "Failed to load transaction history");
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    if (walletAddress) {
      loadTxHistory();
    }
  }, [walletAddress]);

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
      <h2 style={{ fontSize: "1.4rem", marginBottom: "1rem", fontWeight: "600", color: "#f8fafc" }}>
        📜 Transaction History
      </h2>

      {!walletAddress ? (
        <div style={{
          padding: "2rem",
          textAlign: "center",
          color: "#94a3b8",
          fontStyle: "italic"
        }}>
          No wallet loaded. Go to Wallet or Send tab to create/load a wallet.
        </div>
      ) : (
        <>
          <div style={{ marginBottom: "1rem", display: "flex", gap: "1rem", alignItems: "center", flexWrap: "wrap" }}>
            <div style={{ flex: 1, minWidth: "200px" }}>
              <strong style={{ color: "#8b5cf6" }}>Current Wallet:</strong>{" "}
              <span style={{
                fontFamily: "'JetBrains Mono', monospace",
                fontSize: "0.85rem",
                color: "#06b6d4"
              }}>
                {walletAddress}
              </span>
            </div>
            <div style={{ display: "flex", gap: "0.5rem", alignItems: "center" }}>
              <button
                onClick={() => exportTransactions('csv')}
                disabled={!txHistory.length || exporting}
                style={{
                  padding: "0.5rem 1rem",
                  borderRadius: 6,
                  border: "1px solid rgba(99, 102, 241, 0.3)",
                  background: "rgba(99, 102, 241, 0.1)",
                  color: "#94a3b8",
                  cursor: (!txHistory.length || exporting) ? "not-allowed" : "pointer",
                  fontWeight: "500",
                  fontSize: "0.85rem",
                  transition: "all 0.2s",
                  opacity: (!txHistory.length || exporting) ? 0.5 : 1,
                }}
                onMouseEnter={(e) => {
                  if (txHistory.length && !exporting) {
                    e.currentTarget.style.background = "rgba(99, 102, 241, 0.2)";
                    e.currentTarget.style.color = "#e2e8f0";
                  }
                }}
                onMouseLeave={(e) => {
                  e.currentTarget.style.background = "rgba(99, 102, 241, 0.1)";
                  e.currentTarget.style.color = "#94a3b8";
                }}
              >
                📄 Export CSV
              </button>
              <button
                onClick={() => exportTransactions('json')}
                disabled={!txHistory.length || exporting}
                style={{
                  padding: "0.5rem 1rem",
                  borderRadius: 6,
                  border: "1px solid rgba(99, 102, 241, 0.3)",
                  background: "rgba(99, 102, 241, 0.1)",
                  color: "#94a3b8",
                  cursor: (!txHistory.length || exporting) ? "not-allowed" : "pointer",
                  fontWeight: "500",
                  fontSize: "0.85rem",
                  transition: "all 0.2s",
                  opacity: (!txHistory.length || exporting) ? 0.5 : 1,
                }}
                onMouseEnter={(e) => {
                  if (txHistory.length && !exporting) {
                    e.currentTarget.style.background = "rgba(99, 102, 241, 0.2)";
                    e.currentTarget.style.color = "#e2e8f0";
                  }
                }}
                onMouseLeave={(e) => {
                  e.currentTarget.style.background = "rgba(99, 102, 241, 0.1)";
                  e.currentTarget.style.color = "#94a3b8";
                }}
              >
                📋 Export JSON
              </button>
              <button
                onClick={loadTxHistory}
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
                }}
              >
                {loading ? "⏳ Loading..." : "🔄 Refresh"}
              </button>
            </div>
          </div>

          {txHistory.length === 0 ? (
            <div style={{
              padding: "2rem",
              textAlign: "center",
              color: "#94a3b8",
              fontStyle: "italic"
            }}>
              No transactions found for this address.
            </div>
          ) : (
            <div style={{ overflowX: "auto", overflowY: "auto", maxHeight: "600px" }}>
              {txHistory.map((tx) => {
                const isIncoming = tx.direction === "incoming";
                const timestamp = parseInt(tx.timestamp, 16);
                const date = new Date(timestamp * 1000);
                const value = BigInt(tx.value);
                const valueIDAG = (Number(value) / 1e18).toFixed(6);

                return (
                  <div
                    key={tx.hash}
                    style={{
                      padding: "1rem",
                      marginBottom: "0.75rem",
                      borderRadius: 10,
                      background: isIncoming
                        ? "rgba(16, 185, 129, 0.1)"
                        : "rgba(239, 68, 68, 0.1)",
                      border: isIncoming
                        ? "1px solid rgba(16, 185, 129, 0.3)"
                        : "1px solid rgba(239, 68, 68, 0.3)",
                    }}
                  >
                    <div style={{
                      display: "flex",
                      justifyContent: "space-between",
                      alignItems: "center",
                      marginBottom: "0.75rem"
                    }}>
                      <div style={{
                        fontSize: "1.1rem",
                        fontWeight: "600",
                        color: isIncoming ? "#10b981" : "#ef4444"
                      }}>
                        {isIncoming ? "⬇️ Received" : "⬆️ Sent"}
                      </div>
                      <div style={{
                        fontSize: "1.2rem",
                        fontWeight: "700",
                        color: isIncoming ? "#10b981" : "#ef4444"
                      }}>
                        {isIncoming ? "+" : "-"}{valueIDAG} IDAG
                      </div>
                    </div>

                    <div style={{ fontSize: "0.85rem", color: "#94a3b8", marginBottom: "0.5rem" }}>
                      <strong>From:</strong>{" "}
                      <span style={{ fontFamily: "'JetBrains Mono', monospace", color: "#06b6d4" }}>
                        {tx.from}
                      </span>
                    </div>

                    <div style={{ fontSize: "0.85rem", color: "#94a3b8", marginBottom: "0.5rem" }}>
                      <strong>To:</strong>{" "}
                      <span style={{ fontFamily: "'JetBrains Mono', monospace", color: "#06b6d4" }}>
                        {tx.to}
                      </span>
                    </div>

                    <div style={{
                      display: "flex",
                      justifyContent: "space-between",
                      fontSize: "0.85rem",
                      color: "#94a3b8",
                      marginTop: "0.75rem",
                      paddingTop: "0.75rem",
                      borderTop: "1px solid rgba(148, 163, 184, 0.1)"
                    }}>
                      <div>
                        <strong>Block:</strong> {parseInt(tx.block_number, 16)}
                      </div>
                      <div>
                        {date.toLocaleString()}
                      </div>
                    </div>

                    <div style={{
                      fontSize: "0.75rem",
                      color: "#64748b",
                      marginTop: "0.5rem",
                      fontFamily: "'JetBrains Mono', monospace",
                      wordBreak: "break-all"
                    }}>
                      {tx.hash}
                    </div>
                  </div>
                );
              })}
            </div>
          )}
        </>
      )}
    </section>
  );
};
