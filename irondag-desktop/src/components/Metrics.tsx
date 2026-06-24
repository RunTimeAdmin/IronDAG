import React, { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { DagStats, ShardStats, MiningDashboard, ParallelEVMStats } from "../types";
import { LoadingSkeleton } from "./common";

interface MetricsProps {
  setError: (error: string | null) => void;
}

export const Metrics: React.FC<MetricsProps> = ({ setError }) => {
  const [tps, setTps] = useState<string | null>(null);
  const [, setDagStats] = useState<DagStats | null>(null);
  const [shardStats, setShardStats] = useState<ShardStats | null>(null);
  const [, setMiningDashboard] = useState<MiningDashboard | null>(null);
  const [, setParallelEVMStats] = useState<ParallelEVMStats | null>(null);
  const [, setParallelEVMEnabled] = useState<boolean>(false);
  const [loading, setLoading] = useState(false);

  const refreshMetrics = async () => {
    setLoading(true);
    setError(null);
    try {
      const [tpsData, dagData, shardData, miningData] = await Promise.all([
        invoke<any>("get_tps"),
        invoke<DagStats>("get_dag_stats"),
        invoke<ShardStats>("get_shard_stats"),
        invoke<MiningDashboard>("get_mining_dashboard"),
      ]);
      setTps(tpsData);
      setDagStats(dagData);
      setShardStats(shardData);
      setMiningDashboard(miningData);
      await loadParallelEVMStats();
    } catch (e: any) {
      setError(e?.toString?.() ?? "Failed to fetch metrics");
    } finally {
      setLoading(false);
    }
  };

  const loadParallelEVMStats = async () => {
    try {
      const stats = await invoke<ParallelEVMStats>("get_parallel_evm_stats");
      setParallelEVMStats(stats);
      setParallelEVMEnabled(stats?.enabled || false);
    } catch (e: any) {
      // Ignore errors
    }
  };

  useEffect(() => {
    refreshMetrics();
    const interval = setInterval(refreshMetrics, 5000);
    return () => clearInterval(interval);
  }, []);

  return (
    <>
      {/* Network Performance */}
      <section
        style={{
          marginBottom: "1.5rem",
          padding: "1.5rem",
          borderRadius: 16,
          background: "rgba(30, 41, 59, 0.7)",
          backdropFilter: "blur(12px)",
          border: "1px solid rgba(16, 185, 129, 0.2)",
          boxShadow: "0 8px 32px rgba(0, 0, 0, 0.3)",
        }}
      >
        <h2 style={{ fontSize: "1.4rem", marginBottom: "1rem", fontWeight: "600", color: "#f8fafc" }}>
          ⚡ Network Performance
        </h2>
        {loading && !tps ? (
          <div style={{
            marginBottom: "1.5rem",
            padding: "1.5rem",
            background: "rgba(16, 185, 129, 0.15)",
            border: "2px solid rgba(16, 185, 129, 0.3)",
            borderRadius: 12,
          }}>
            <LoadingSkeleton lines={2} width={["medium", "short"]} />
          </div>
        ) : (
          <div style={{
            marginBottom: "1.5rem",
            padding: "1.5rem",
            background: "rgba(16, 185, 129, 0.15)",
            border: "2px solid rgba(16, 185, 129, 0.3)",
            borderRadius: 12,
            textAlign: "center"
          }}>
            <div style={{
              fontSize: "3.5rem",
              fontWeight: "800",
              color: "#10b981",
              textShadow: "0 0 20px rgba(16, 185, 129, 0.5)",
              marginBottom: "0.5rem"
            }}>
              {tps ? `${tps}` : "--"}
            </div>
            <div style={{
              color: "#94a3b8",
              fontSize: "1rem",
              fontWeight: "500",
            }}>
              🚀 TRANSACTIONS PER SECOND
            </div>
          </div>
        )}
        <button
          onClick={refreshMetrics}
          disabled={loading}
          style={{
            padding: "0.65rem 1.5rem",
            borderRadius: 8,
            border: "none",
            background: loading ? "rgba(99, 102, 241, 0.5)" : "linear-gradient(135deg, #6366f1, #4f46e5)",
            color: "white",
            cursor: loading ? "not-allowed" : "pointer",
            fontWeight: "600",
            fontSize: "0.95rem",
          }}
        >
          {loading ? "⏳ Refreshing..." : "🔄 Refresh"}
        </button>
      </section>

      {/* Shard Statistics */}
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
          🧩 Shard Statistics
        </h2>
        {shardStats && shardStats.shard_count > 0 ? (
          <div style={{ display: "grid", gridTemplateColumns: "repeat(auto-fill, minmax(260px, 1fr))", gap: "1rem" }}>
            {shardStats.shards.map((shard, idx) => {
              const shardColors = [
                { bg: "rgba(16, 185, 129, 0.1)", border: "rgba(16, 185, 129, 0.3)", text: "#10b981" },
                { bg: "rgba(139, 92, 246, 0.1)", border: "rgba(139, 92, 246, 0.3)", text: "#8b5cf6" },
                { bg: "rgba(236, 72, 153, 0.1)", border: "rgba(236, 72, 153, 0.3)", text: "#ec4899" },
                { bg: "rgba(6, 182, 212, 0.1)", border: "rgba(6, 182, 212, 0.3)", text: "#06b6d4" },
              ];
              const color = shardColors[idx % shardColors.length];
              return (
                <div
                  key={shard.shard_id}
                  style={{
                    padding: "1rem",
                    borderRadius: 10,
                    background: color.bg,
                    border: `1px solid ${color.border}`,
                  }}
                >
                  <div style={{ fontWeight: "700", marginBottom: "0.75rem", fontSize: "1.1rem", color: color.text }}>
                    📦 Shard #{shard.shard_id}
                  </div>
                  <div style={{ display: "flex", flexDirection: "column", gap: "0.5rem", fontSize: "0.9rem" }}>
                    <div style={{ display: "flex", justifyContent: "space-between" }}>
                      <span style={{ color: "#94a3b8" }}>Blocks:</span>
                      <strong style={{ color: "#f8fafc" }}>{shard.block_count}</strong>
                    </div>
                    <div style={{ display: "flex", justifyContent: "space-between" }}>
                      <span style={{ color: "#94a3b8" }}>Pending Txs:</span>
                      <strong style={{ color: "#fbbf24" }}>{shard.transaction_pool_size}</strong>
                    </div>
                  </div>
                </div>
              );
            })}
          </div>
        ) : (
          <div style={{
            padding: "2rem",
            textAlign: "center",
            background: "rgba(251, 191, 36, 0.1)",
            border: "1px solid rgba(251, 191, 36, 0.2)",
            borderRadius: 12
          }}>
            <p style={{ color: "#fbbf24", fontSize: "1.1rem", fontStyle: "italic" }}>
              ⚠️ Sharding not enabled or no shard data available.
            </p>
          </div>
        )}
      </section>
    </>
  );
};
