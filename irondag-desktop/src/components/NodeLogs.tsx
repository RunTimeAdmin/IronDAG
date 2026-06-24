import React, { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { NodeProcessInfo } from "../types";

interface NodeLogsProps {
  setError: (error: string | null) => void;
}

export const NodeLogs: React.FC<NodeLogsProps> = ({ setError }) => {
  const [nodeProcesses, setNodeProcesses] = useState<NodeProcessInfo[]>([]);
  const [logNodeId, setLogNodeId] = useState<string>("");
  const [logLines, setLogLines] = useState<string[]>([]);
  const [logAutoRefresh, setLogAutoRefresh] = useState<boolean>(true);

  useEffect(() => {
    loadNodeProcesses();
  }, []);

  useEffect(() => {
    if (!logNodeId || !logAutoRefresh) return;
    const interval = setInterval(() => {
      loadNodeLogs(logNodeId);
    }, 2000);
    return () => clearInterval(interval);
  }, [logNodeId, logAutoRefresh]);

  const loadNodeProcesses = async () => {
    try {
      const processes = await invoke<NodeProcessInfo[]>("get_node_processes");
      setNodeProcesses(processes);
    } catch (e: any) {
      setError(e?.toString?.() ?? "Failed to load node processes");
    }
  };

  const loadNodeLogs = async (id: string) => {
    try {
      const lines = await invoke<string[]>("get_node_logs", { nodeId: id, maxLines: 200 });
      setLogLines(lines.reverse());
    } catch (e: any) {
      setError(e?.toString?.() ?? "Failed to load logs");
    }
  };

  const clearNodeLogs = async (id: string) => {
    try {
      await invoke("clear_node_logs", { nodeId: id });
      setLogLines([]);
    } catch (e: any) {
      setError(e?.toString?.() ?? "Failed to clear logs");
    }
  };

  return (
    <section
      style={{
        padding: "1.5rem",
        borderRadius: 16,
        background: "rgba(30, 41, 59, 0.7)",
        backdropFilter: "blur(12px)",
        border: "1px solid rgba(59, 130, 246, 0.2)",
        boxShadow: "0 8px 32px rgba(0, 0, 0, 0.3)",
      }}
    >
      <h2 style={{ fontSize: "1.4rem", marginBottom: "1rem", fontWeight: "600", color: "#f8fafc" }}>
        📜 Node Logs
      </h2>

      <div style={{ display: "flex", gap: "0.75rem", flexWrap: "wrap", alignItems: "center", marginBottom: "1rem" }}>
        <select
          value={logNodeId}
          onChange={(e) => {
            const selected = e.target.value;
            setLogNodeId(selected);
            if (selected) {
              loadNodeLogs(selected);
            }
          }}
          style={{
            padding: "0.55rem",
            borderRadius: 8,
            border: "1px solid rgba(59, 130, 246, 0.3)",
            background: "rgba(2, 6, 23, 0.6)",
            color: "#e5e7eb",
            minWidth: 200,
          }}
        >
          <option value="">Select node</option>
          {nodeProcesses.map((node) => (
            <option key={node.id} value={node.id}>
              {node.id}
            </option>
          ))}
        </select>
        <button
          onClick={() => logNodeId && loadNodeLogs(logNodeId)}
          disabled={!logNodeId}
          style={{
            padding: "0.55rem 1rem",
            borderRadius: 8,
            border: "none",
            background: "rgba(148, 163, 184, 0.3)",
            color: "white",
            cursor: !logNodeId ? "not-allowed" : "pointer",
            fontWeight: "600",
          }}
        >
          🔄 Refresh
        </button>
        <button
          onClick={() => logNodeId && clearNodeLogs(logNodeId)}
          disabled={!logNodeId}
          style={{
            padding: "0.55rem 1rem",
            borderRadius: 8,
            border: "none",
            background: "rgba(239, 68, 68, 0.5)",
            color: "white",
            cursor: !logNodeId ? "not-allowed" : "pointer",
            fontWeight: "600",
          }}
        >
          🧹 Clear
        </button>
        <label style={{ color: "#e2e8f0", display: "flex", alignItems: "center", gap: "0.4rem" }}>
          <input
            type="checkbox"
            checked={logAutoRefresh}
            onChange={(e) => setLogAutoRefresh(e.target.checked)}
          />
          Auto refresh
        </label>
      </div>

      <pre
        style={{
          padding: "0.75rem",
          borderRadius: 8,
          background: "rgba(2, 6, 23, 0.7)",
          color: "#e2e8f0",
          maxHeight: 400,
          overflow: "auto",
          whiteSpace: "pre-wrap",
          fontSize: "0.85rem",
          fontFamily: "'JetBrains Mono', monospace",
        }}
      >
        {logLines.length > 0 ? logLines.join("\n") : "(no logs loaded)"}
      </pre>
    </section>
  );
};
