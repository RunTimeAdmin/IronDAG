import React, { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { NodeStatus, MiningStatus, NodeProcessInfo } from "../types";
import { useRpc } from "../hooks/useRpc";
import type { ToastType } from "./common/Toast";

interface DashboardProps {
  nodeStatus: NodeStatus | null;
  isRefreshing: boolean;
  onRefresh: () => void;
  setError: (error: string | null) => void;
  setConfirmDialog: (dialog: { title: string; message: string; onConfirm: () => void } | null) => void;
  addToast?: (message: string, type: ToastType) => void;
}

export const Dashboard: React.FC<DashboardProps> = ({
  nodeStatus,
  isRefreshing,
  onRefresh,
  setError,
  setConfirmDialog,
  addToast
}) => {
  const [miningStatus, setMiningStatus] = useState<MiningStatus | null>(null);
  const [nodeProcesses, setNodeProcesses] = useState<NodeProcessInfo[]>([]);
  const [nodeId, setNodeId] = useState<string>("node1");
  const [p2pPort, setP2pPort] = useState<string>("8080");
  const [rpcPort, setRpcPort] = useState<string>("9090");
  const [httpPort, setHttpPort] = useState<string>("8081");
  const [dataDir, setDataDir] = useState<string>("data-node1");
  const [enableMining, setEnableMining] = useState<boolean>(true);
  const [noTestTxs, setNoTestTxs] = useState<boolean>(true);
  const [peerList, setPeerList] = useState<string>("127.0.0.1:8081,127.0.0.1:8082");
  const [logStreamingEnabled, setLogStreamingEnabled] = useState<boolean>(true);
  const [resettingDataDir, setResettingDataDir] = useState<boolean>(false);
  const [rpcUrl, setRpcUrl] = useState<string>("http://127.0.0.1:8082");
  const [loading, setLoading] = useState(false);

  const { execute: startMining } = useRpc("start_mining");
  const { execute: stopMining } = useRpc("stop_mining");

  // Load mining status
  useEffect(() => {
    const loadMiningStatus = async () => {
      try {
        const status = await invoke<MiningStatus>("get_mining_status");
        setMiningStatus(status);
      } catch (e) {
        // Ignore errors
      }
    };
    loadMiningStatus();
  }, [nodeStatus]);

  // Load node processes
  const loadNodeProcesses = async () => {
    try {
      const processes = await invoke<NodeProcessInfo[]>("get_node_processes");
      setNodeProcesses(processes);
      const failed = processes.filter((p) => p.exit_code !== null);
      if (failed.length > 0) {
        const first = failed[0];
        setError(
          `Node ${first.id} exited (code ${first.exit_code ?? "unknown"}). Check logs; if genesis mismatch, click Reset Data Dir.`
        );
      }
    } catch (e: any) {
      setError(e?.toString?.() ?? "Failed to load node processes");
    }
  };

  useEffect(() => {
    loadNodeProcesses();
    // Load current RPC URL
    invoke<string>("get_rpc_url").then(setRpcUrl).catch(() => {});
  }, []);

  const handleStartMining = async () => {
    setLoading(true);
    try {
      await startMining();
      const status = await invoke<MiningStatus>("get_mining_status");
      setMiningStatus(status);
      onRefresh();
    } catch (e: any) {
      setError(e?.toString?.() ?? "Failed to start mining");
    } finally {
      setLoading(false);
    }
  };

  const handleStopMining = async () => {
    setLoading(true);
    try {
      await stopMining();
      const status = await invoke<MiningStatus>("get_mining_status");
      setMiningStatus(status);
      onRefresh();
    } catch (e: any) {
      setError(e?.toString?.() ?? "Failed to stop mining");
    } finally {
      setLoading(false);
    }
  };

  const handleStartNode = async () => {
    setLoading(true);
    try {
      const peers = peerList
        .split(",")
        .map((peer) => peer.trim())
        .filter(Boolean);
      await invoke<NodeProcessInfo>("start_node", {
        nodeId,
        p2pPort: Number(p2pPort),
        rpcPort: Number(rpcPort),
        httpPort: httpPort ? Number(httpPort) : null,
        dataDir: dataDir || null,
        enableMining,
        peers: peers.length > 0 ? peers : null,
        logStreaming: logStreamingEnabled,
        noTestTxs,
      });
      await loadNodeProcesses();
      const newRpcUrl = await invoke<string>("get_rpc_url");
      setRpcUrl(newRpcUrl);
      setTimeout(onRefresh, 1000);
      addToast?.("Node started successfully", "success");
    } catch (e: any) {
      setError(e?.toString?.() ?? "Failed to start node");
    } finally {
      setLoading(false);
    }
  };

  const handleStopNode = async (id: string) => {
    setLoading(true);
    try {
      await invoke("stop_node", { nodeId: id });
      await loadNodeProcesses();
      addToast?.("Node stopped successfully", "info");
    } catch (e: any) {
      setError(e?.toString?.() ?? "Failed to stop node");
    } finally {
      setLoading(false);
    }
  };

  const handleStopAllNodes = async () => {
    setLoading(true);
    try {
      await invoke("stop_all_nodes");
      await loadNodeProcesses();
    } catch (e: any) {
      setError(e?.toString?.() ?? "Failed to stop all nodes");
    } finally {
      setLoading(false);
    }
  };

  const handleResetDataDir = async () => {
    setResettingDataDir(true);
    try {
      await invoke("reset_data_dir", { dataDir });
      await loadNodeProcesses();
    } catch (e: any) {
      setError(e?.toString?.() ?? "Failed to reset data directory");
    } finally {
      setResettingDataDir(false);
    }
  };

  const handleSetLogStreaming = async (id: string, enabled: boolean) => {
    try {
      await invoke("set_node_log_streaming", { nodeId: id, enabled });
      await loadNodeProcesses();
    } catch (e: any) {
      setError(e?.toString?.() ?? "Failed to update log streaming");
    }
  };

  const updateRpcUrl = async (newUrl: string) => {
    setRpcUrl(newUrl);
    try {
      await invoke("set_rpc_url", { newUrl });
    } catch (e) {
      console.error("Failed to set RPC URL:", e);
    }
  };

  const miningOn = nodeStatus?.is_mining ?? false;

  return (
    <>
      {/* Node Status Section */}
      <section
        style={{
          marginBottom: "1.5rem",
          padding: "1.5rem",
          borderRadius: 16,
          background: "rgba(30, 41, 59, 0.7)",
          backdropFilter: "blur(12px)",
          border: "1px solid rgba(99, 102, 241, 0.2)",
          boxShadow: "0 8px 32px rgba(0, 0, 0, 0.3)",
        }}
        aria-live="polite"
        aria-label="Node status information"
      >
        <h2 style={{ fontSize: "1.4rem", marginBottom: "1rem", fontWeight: "600", color: "#f8fafc" }}>
          Node Status
        </h2>
        {nodeStatus ? (
          <ul style={{ listStyle: "none", paddingLeft: 0, marginBottom: "1rem" }}>
            <li style={{ marginBottom: "0.75rem", display: "flex", alignItems: "center", gap: "0.5rem" }}>
              <strong style={{ color: "#94a3b8", minWidth: "140px" }}>RPC Endpoint</strong>
              <input
                type="text"
                value={rpcUrl}
                onChange={(e) => updateRpcUrl(e.target.value)}
                aria-label="RPC endpoint URL"
                style={{
                  flex: 1,
                  padding: "0.4rem 0.6rem",
                  borderRadius: 6,
                  border: "1px solid rgba(167, 139, 250, 0.3)",
                  background: "rgba(2, 6, 23, 0.6)",
                  color: "#a78bfa",
                  fontWeight: "500",
                  fontSize: "0.9rem",
                  fontFamily: "monospace",
                }}
              />
              {isRefreshing && <span style={{ color: "#64748b", fontSize: "0.8rem" }} aria-hidden="true">⟳</span>}
            </li>
            <li style={{ marginBottom: "0.75rem", display: "flex", alignItems: "center", gap: "0.5rem" }}>
              <strong style={{ color: "#94a3b8", minWidth: "140px" }}>Height</strong>
              <span style={{ color: "#06b6d4", fontWeight: "600", fontSize: "1.05rem" }}>{nodeStatus.height}</span>
            </li>
            <li style={{ marginBottom: "0.75rem", display: "flex", alignItems: "center", gap: "0.5rem" }}>
              <strong style={{ color: "#94a3b8", minWidth: "140px" }}>Total Transactions</strong>
              <span style={{ color: "#06b6d4", fontWeight: "600", fontSize: "1.05rem" }}>{nodeStatus.tx_count}</span>
            </li>
            <li style={{ marginBottom: "0.75rem", display: "flex", alignItems: "center", gap: "0.5rem" }}>
              <strong style={{ color: "#94a3b8", minWidth: "140px" }}>Connected Peers</strong>
              <span style={{ color: "#06b6d4", fontWeight: "600", fontSize: "1.05rem" }}>{nodeStatus.peer_count}</span>
            </li>
            <li style={{ marginBottom: "0.75rem", display: "flex", alignItems: "center", gap: "0.5rem" }}>
              <strong style={{ color: "#94a3b8", minWidth: "140px" }}>Mining</strong>
              <span
                style={{
                  color: miningOn ? "#10b981" : "#64748b",
                  fontWeight: "700",
                  fontSize: "1.05rem",
                  textShadow: miningOn ? "0 0 10px rgba(16, 185, 129, 0.5)" : "none"
                }}
                aria-label={`Mining status: ${miningOn ? 'running' : 'stopped'}`}
              >
                {miningOn ? "ON" : "OFF"}
              </span>
            </li>
          </ul>
        ) : (
          <div style={{
            padding: "1.5rem",
            background: "rgba(59, 130, 246, 0.1)",
            borderRadius: 12,
            border: "1px solid rgba(59, 130, 246, 0.3)",
            textAlign: "center"
          }}>
            <div style={{ fontSize: "2rem", marginBottom: "0.5rem" }}>🔌</div>
            <p style={{ color: "#94a3b8", fontStyle: "italic", marginBottom: "0.75rem" }}>
              Not connected to node
            </p>
            <div style={{ marginBottom: "1rem", textAlign: "left" }}>
              <label style={{ color: "#94a3b8", display: "block", marginBottom: "0.4rem", fontSize: "0.9rem" }}>
                RPC Endpoint
              </label>
              <div style={{ display: "flex", gap: "0.5rem" }}>
                <input
                  type="text"
                  value={rpcUrl}
                  onChange={(e) => setRpcUrl(e.target.value)}
                  placeholder="http://127.0.0.1:8082"
                  aria-label="RPC endpoint URL"
                  style={{
                    flex: 1,
                    padding: "0.6rem 0.8rem",
                    borderRadius: 8,
                    border: "1px solid rgba(99, 102, 241, 0.3)",
                    background: "rgba(2, 6, 23, 0.6)",
                    color: "#a78bfa",
                    fontWeight: "500",
                    fontSize: "0.9rem",
                    fontFamily: "monospace",
                  }}
                />
                <button
                  onClick={async () => {
                    setLoading(true);
                    try {
                      await invoke("set_rpc_url", { newUrl: rpcUrl });
                      await onRefresh();
                    } catch (e: any) {
                      setError(e?.toString?.() ?? "Connection failed");
                    } finally {
                      setLoading(false);
                    }
                  }}
                  disabled={loading}
                  aria-label="Connect to node"
                  style={{
                    padding: "0.6rem 1.2rem",
                    borderRadius: 8,
                    border: "none",
                    background: loading ? "rgba(16, 185, 129, 0.5)" : "linear-gradient(135deg, #10b981, #059669)",
                    color: "white",
                    cursor: loading ? "not-allowed" : "pointer",
                    fontWeight: "600",
                    fontSize: "0.9rem",
                  }}
                >
                  {loading ? "⏳" : "🔗"} Connect
                </button>
              </div>
            </div>
            <p style={{ color: "#64748b", fontSize: "0.85rem" }}>
              Default: http://127.0.0.1:8082 — or start a node below
            </p>
          </div>
        )}
        <button
          onClick={onRefresh}
          disabled={loading || isRefreshing}
          aria-label="Refresh node status"
          style={{
            marginTop: "0.25rem",
            padding: "0.65rem 1.5rem",
            borderRadius: 8,
            border: "none",
            background: loading || isRefreshing ? "rgba(99, 102, 241, 0.5)" : "linear-gradient(135deg, #6366f1, #4f46e5)",
            color: "white",
            cursor: loading || isRefreshing ? "not-allowed" : "pointer",
            fontWeight: "600",
            fontSize: "0.95rem",
            boxShadow: loading || isRefreshing ? "none" : "0 4px 12px rgba(99, 102, 241, 0.3)",
            transition: "all 0.3s ease",
            opacity: loading || isRefreshing ? 0.6 : 1,
          }}
        >
          {loading || isRefreshing ? "⏳ Refreshing..." : "🔄 Refresh"}
        </button>
      </section>

      {/* Mining Section */}
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
          TriStream Mining
        </h2>
        <div style={{ marginBottom: "1rem", display: "flex", gap: "0.75rem" }}>
          <button
            onClick={handleStartMining}
            disabled={loading || miningOn}
            aria-label="Start mining"
            style={{
              padding: "0.65rem 1.5rem",
              borderRadius: 8,
              border: "none",
              background: miningOn ? "rgba(75, 85, 99, 0.5)" : "linear-gradient(135deg, #10b981, #059669)",
              color: "white",
              cursor: miningOn || loading ? "not-allowed" : "pointer",
              fontWeight: "600",
              fontSize: "0.95rem",
              boxShadow: miningOn || loading ? "none" : "0 4px 12px rgba(16, 185, 129, 0.3)",
              transition: "all 0.3s ease",
              opacity: miningOn || loading ? 0.5 : 1,
            }}
          >
            {loading ? "⏳" : "▶️"} Start Mining
          </button>
          <button
            onClick={handleStopMining}
            disabled={loading || !miningOn}
            aria-label="Stop mining"
            style={{
              padding: "0.65rem 1.5rem",
              borderRadius: 8,
              border: "none",
              background: !miningOn ? "rgba(75, 85, 99, 0.5)" : "linear-gradient(135deg, #ef4444, #b91c1c)",
              color: "white",
              cursor: !miningOn || loading ? "not-allowed" : "pointer",
              fontWeight: "600",
              fontSize: "0.95rem",
              boxShadow: !miningOn || loading ? "none" : "0 4px 12px rgba(239, 68, 68, 0.3)",
              transition: "all 0.3s ease",
              opacity: !miningOn || loading ? 0.5 : 1,
            }}
          >
            {loading ? "⏳" : "⏹️"} Stop Mining
          </button>
        </div>

        {miningStatus && (
          <>
            <div style={{
              marginBottom: "1rem",
              padding: "0.75rem 1rem",
              background: "rgba(99, 102, 241, 0.1)",
              borderRadius: 8,
              border: "1px solid rgba(99, 102, 241, 0.2)"
            }}>
              <strong style={{ color: "#94a3b8" }}>Pending Transactions</strong>:{" "}
              <span style={{ color: "#06b6d4", fontWeight: "700", fontSize: "1.1rem" }}>
                {miningStatus.pending_txs}
              </span>
            </div>
            <h3 style={{ marginTop: "0.5rem", marginBottom: "0.75rem", fontSize: "1.1rem", color: "#e2e8f0" }}>
              Stream Configuration
            </h3>
            <ul style={{ listStyle: "none", paddingLeft: 0, fontSize: "0.95rem" }}>
              <li style={{
                marginBottom: "0.5rem",
                padding: "0.75rem",
                background: "rgba(16, 185, 129, 0.1)",
                borderRadius: 8,
                border: "1px solid rgba(16, 185, 129, 0.2)"
              }}>
                <strong style={{ color: "#10b981" }}>Stream A</strong>:
                <span style={{ color: "#94a3b8" }}> {miningStatus.streams.streamA.max_txs} tx / {miningStatus.streams.streamA.block_time_ms} ms</span>
                <span style={{ color: "#fbbf24", marginLeft: "0.5rem" }}>💰 {miningStatus.streams.streamA.reward}</span>
              </li>
              <li style={{
                marginBottom: "0.5rem",
                padding: "0.75rem",
                background: "rgba(139, 92, 246, 0.1)",
                borderRadius: 8,
                border: "1px solid rgba(139, 92, 246, 0.2)"
              }}>
                <strong style={{ color: "#8b5cf6" }}>Stream B</strong>:
                <span style={{ color: "#94a3b8" }}> {miningStatus.streams.streamB.max_txs} tx / {miningStatus.streams.streamB.block_time_ms} ms</span>
                <span style={{ color: "#fbbf24", marginLeft: "0.5rem" }}>💰 {miningStatus.streams.streamB.reward}</span>
              </li>
              <li style={{
                marginBottom: "0.5rem",
                padding: "0.75rem",
                background: "rgba(236, 72, 153, 0.1)",
                borderRadius: 8,
                border: "1px solid rgba(236, 72, 153, 0.2)"
              }}>
                <strong style={{ color: "#ec4899" }}>Stream C</strong>:
                <span style={{ color: "#94a3b8" }}> {miningStatus.streams.streamC.max_txs} tx / {miningStatus.streams.streamC.block_time_ms} ms</span>
                <span style={{ color: "#fbbf24", marginLeft: "0.5rem" }}>💰 {miningStatus.streams.streamC.reward}</span>
              </li>
            </ul>
          </>
        )}
      </section>

      {/* Node Control Section */}
      <section
        style={{
          marginTop: "1.5rem",
          padding: "1.5rem",
          borderRadius: 16,
          background: "rgba(30, 41, 59, 0.7)",
          backdropFilter: "blur(12px)",
          border: "1px solid rgba(59, 130, 246, 0.2)",
          boxShadow: "0 8px 32px rgba(0, 0, 0, 0.3)",
        }}
      >
        <h2 style={{ fontSize: "1.4rem", marginBottom: "1rem", fontWeight: "600", color: "#f8fafc" }}>
          Node Control
        </h2>
        <div style={{ display: "grid", gap: "0.75rem", gridTemplateColumns: "repeat(auto-fit, minmax(200px, 1fr))" }}>
          <div>
            <label style={{ color: "#94a3b8", display: "block", marginBottom: "0.4rem" }}>Node ID</label>
            <input
              value={nodeId}
              onChange={(e) => setNodeId(e.target.value)}
              placeholder="node1"
              aria-label="Node ID"
              style={{
                width: "100%",
                padding: "0.65rem",
                borderRadius: 8,
                border: "1px solid rgba(59, 130, 246, 0.3)",
                background: "rgba(2, 6, 23, 0.6)",
                color: "#e5e7eb",
              }}
            />
          </div>
          <div>
            <label style={{ color: "#94a3b8", display: "block", marginBottom: "0.4rem" }}>P2P Port</label>
            <input
              value={p2pPort}
              onChange={(e) => setP2pPort(e.target.value)}
              placeholder="8080"
              aria-label="P2P port number"
              style={{
                width: "100%",
                padding: "0.65rem",
                borderRadius: 8,
                border: "1px solid rgba(59, 130, 246, 0.3)",
                background: "rgba(2, 6, 23, 0.6)",
                color: "#e5e7eb",
              }}
            />
          </div>
          <div>
            <label style={{ color: "#94a3b8", display: "block", marginBottom: "0.4rem" }}>RPC Port</label>
            <input
              value={rpcPort}
              onChange={(e) => setRpcPort(e.target.value)}
              placeholder="9090"
              aria-label="RPC port number"
              style={{
                width: "100%",
                padding: "0.65rem",
                borderRadius: 8,
                border: "1px solid rgba(59, 130, 246, 0.3)",
                background: "rgba(2, 6, 23, 0.6)",
                color: "#e5e7eb",
              }}
            />
          </div>
          <div>
            <label style={{ color: "#94a3b8", display: "block", marginBottom: "0.4rem" }}>HTTP Port</label>
            <input
              value={httpPort}
              onChange={(e) => setHttpPort(e.target.value)}
              placeholder="8081"
              aria-label="HTTP port number"
              style={{
                width: "100%",
                padding: "0.65rem",
                borderRadius: 8,
                border: "1px solid rgba(59, 130, 246, 0.3)",
                background: "rgba(2, 6, 23, 0.6)",
                color: "#e5e7eb",
              }}
            />
          </div>
          <div>
            <label style={{ color: "#94a3b8", display: "block", marginBottom: "0.4rem" }}>Data Dir</label>
            <input
              value={dataDir}
              onChange={(e) => setDataDir(e.target.value)}
              placeholder="data-node1"
              aria-label="Data directory path"
              style={{
                width: "100%",
                padding: "0.65rem",
                borderRadius: 8,
                border: "1px solid rgba(59, 130, 246, 0.3)",
                background: "rgba(2, 6, 23, 0.6)",
                color: "#e5e7eb",
              }}
            />
            <button
              onClick={() => setConfirmDialog({
                title: "Reset Data",
                message: "This will delete all blockchain data. Are you sure?",
                onConfirm: handleResetDataDir
              })}
              disabled={resettingDataDir || loading}
              aria-label="Reset data directory"
              style={{
                marginTop: "0.5rem",
                padding: "0.55rem 1rem",
                borderRadius: 8,
                border: "none",
                background: resettingDataDir
                  ? "rgba(239, 68, 68, 0.4)"
                  : "linear-gradient(135deg, #ef4444, #b91c1c)",
                color: "white",
                cursor: resettingDataDir || loading ? "not-allowed" : "pointer",
                fontWeight: "600",
                fontSize: "0.9rem",
                width: "100%",
                opacity: resettingDataDir || loading ? 0.7 : 1,
              }}
            >
              {resettingDataDir ? "⏳ Resetting..." : "🧹 Reset Data Dir"}
            </button>
          </div>
          <div style={{ gridColumn: "1 / -1" }}>
            <label style={{ color: "#94a3b8", display: "block", marginBottom: "0.4rem" }}>Peers (comma-separated)</label>
            <input
              value={peerList}
              onChange={(e) => setPeerList(e.target.value)}
              placeholder="127.0.0.1:8081,127.0.0.1:8082"
              aria-label="Peer list comma separated"
              style={{
                width: "100%",
                padding: "0.65rem",
                borderRadius: 8,
                border: "1px solid rgba(59, 130, 246, 0.3)",
                background: "rgba(2, 6, 23, 0.6)",
                color: "#e5e7eb",
              }}
            />
          </div>
          <div style={{ display: "flex", alignItems: "center", gap: "0.5rem" }}>
            <input
              type="checkbox"
              checked={enableMining}
              onChange={(e) => setEnableMining(e.target.checked)}
              aria-label="Enable mining"
            />
            <span style={{ color: "#e2e8f0" }}>Enable mining</span>
          </div>
          <div style={{ display: "flex", alignItems: "center", gap: "0.5rem" }}>
            <input
              type="checkbox"
              checked={noTestTxs}
              onChange={(e) => setNoTestTxs(e.target.checked)}
              aria-label="Disable test transaction generation"
            />
            <span style={{ color: "#e2e8f0" }}>Disable test tx generation</span>
          </div>
          <div style={{ display: "flex", alignItems: "center", gap: "0.5rem" }}>
            <input
              type="checkbox"
              checked={logStreamingEnabled}
              onChange={(e) => setLogStreamingEnabled(e.target.checked)}
              aria-label="Enable log streaming"
            />
            <span style={{ color: "#e2e8f0" }}>Stream logs</span>
          </div>
        </div>

        <div style={{ marginTop: "1rem", display: "flex", gap: "0.75rem", flexWrap: "wrap" }}>
          <button
            onClick={handleStartNode}
            disabled={loading}
            aria-label="Start blockchain node"
            style={{
              padding: "0.65rem 1.5rem",
              borderRadius: 8,
              border: "none",
              background: "linear-gradient(135deg, #3b82f6, #2563eb)",
              color: "white",
              cursor: loading ? "not-allowed" : "pointer",
              fontWeight: "600",
            }}
          >
            🚀 Start Node
          </button>
          <button
            onClick={loadNodeProcesses}
            disabled={loading}
            aria-label="Refresh node processes"
            style={{
              padding: "0.65rem 1.5rem",
              borderRadius: 8,
              border: "none",
              background: "rgba(148, 163, 184, 0.3)",
              color: "white",
              cursor: loading ? "not-allowed" : "pointer",
              fontWeight: "600",
            }}
          >
            🔄 Refresh
          </button>
          <button
            onClick={handleStopAllNodes}
            disabled={loading || nodeProcesses.length === 0}
            aria-label="Stop all nodes"
            style={{
              padding: "0.65rem 1.5rem",
              borderRadius: 8,
              border: "none",
              background: nodeProcesses.length === 0 ? "rgba(75, 85, 99, 0.5)" : "linear-gradient(135deg, #ef4444, #b91c1c)",
              color: "white",
              cursor: loading || nodeProcesses.length === 0 ? "not-allowed" : "pointer",
              fontWeight: "600",
              opacity: loading || nodeProcesses.length === 0 ? 0.5 : 1,
            }}
          >
            🧯 Stop All
          </button>
        </div>

        {/* Running Nodes */}
        <div style={{ marginTop: "1.25rem" }}>
          <h3 style={{ marginBottom: "0.75rem", color: "#e2e8f0", fontSize: "1.05rem" }}>Running Nodes</h3>
          {nodeProcesses.length === 0 ? (
            <p style={{ color: "#94a3b8", fontStyle: "italic" }}>No managed nodes running.</p>
          ) : (
            <div style={{ display: "grid", gap: "0.75rem" }}>
              {nodeProcesses.map((node) => (
                <div
                  key={node.id}
                  style={{
                    padding: "0.75rem 1rem",
                    borderRadius: 10,
                    background: "rgba(15, 23, 42, 0.6)",
                    border: "1px solid rgba(59, 130, 246, 0.2)",
                    display: "flex",
                    justifyContent: "space-between",
                    alignItems: "center",
                    gap: "1rem",
                    flexWrap: "wrap",
                  }}
                >
                  <div style={{ color: "#e2e8f0" }}>
                    <strong>{node.id}</strong> (pid {node.pid}) · P2P {node.p2p_port} · RPC {node.rpc_port}
                    {node.http_port ? ` · HTTP ${node.http_port}` : ""}
                    {node.data_dir ? ` · ${node.data_dir}` : ""}
                  </div>
                  <div style={{ display: "flex", gap: "0.5rem", flexWrap: "wrap" }}>
                    <button
                      onClick={() => handleSetLogStreaming(node.id, !node.log_streaming)}
                      disabled={loading}
                      aria-label={node.log_streaming ? `Disable log streaming for ${node.id}` : `Enable log streaming for ${node.id}`}
                      style={{
                        padding: "0.45rem 0.9rem",
                        borderRadius: 8,
                        border: "none",
                        background: node.log_streaming ? "linear-gradient(135deg, #f59e0b, #d97706)" : "rgba(71, 85, 105, 0.6)",
                        color: "white",
                        cursor: loading ? "not-allowed" : "pointer",
                        fontWeight: "600",
                      }}
                    >
                      {node.log_streaming ? "🟡 Logs On" : "⚪ Logs Off"}
                    </button>
                    <button
                      onClick={() => setConfirmDialog({
                        title: "Stop Node",
                        message: `Stop the blockchain node ${node.id}?`,
                        onConfirm: () => handleStopNode(node.id)
                      })}
                      disabled={loading}
                      aria-label={`Stop node ${node.id}`}
                      style={{
                        padding: "0.45rem 0.9rem",
                        borderRadius: 8,
                        border: "none",
                        background: "linear-gradient(135deg, #ef4444, #b91c1c)",
                        color: "white",
                        cursor: loading ? "not-allowed" : "pointer",
                        fontWeight: "600",
                      }}
                    >
                      ⏹️ Stop
                    </button>
                  </div>
                </div>
              ))}
            </div>
          )}
        </div>
      </section>
    </>
  );
};
