import React, { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { Block, DagStats } from "../types";
import { StatCardSkeleton } from "./common";

interface ExplorerProps {
  setError: (error: string | null) => void;
}

export const Explorer: React.FC<ExplorerProps> = ({ setError }) => {
  const [blocks, setBlocks] = useState<Block[]>([]);
  const [dagStats, setDagStats] = useState<DagStats | null>(null);
  const [selectedBlock, setSelectedBlock] = useState<Block | null>(null);
  const [dagViewMode, setDagViewMode] = useState<"list" | "graph">("graph");
  const [dagZoom, setDagZoom] = useState<number>(1);
  const [dagPan, setDagPan] = useState<{ x: number; y: number }>({ x: 0, y: 0 });
  const [loading, setLoading] = useState(false);

  const refreshExplorer = async () => {
    setLoading(true);
    setError(null);
    try {
      const [blocksData, statsData] = await Promise.all([
        invoke<any>("get_latest_blocks", { count: 10 }),
        invoke<DagStats>("get_dag_stats"),
      ]);
      setBlocks(blocksData);
      setDagStats(statsData);
    } catch (e: any) {
      setError(e?.toString?.() ?? "Failed to fetch explorer data");
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    refreshExplorer();
    const interval = setInterval(refreshExplorer, 10000);
    return () => clearInterval(interval);
  }, []);

  const streamColors: Record<string, { bg: string; border: string; text: string }> = {
    A: { bg: "rgba(16, 185, 129, 0.1)", border: "rgba(16, 185, 129, 0.3)", text: "#10b981" },
    B: { bg: "rgba(139, 92, 246, 0.1)", border: "rgba(139, 92, 246, 0.3)", text: "#8b5cf6" },
    C: { bg: "rgba(236, 72, 153, 0.1)", border: "rgba(236, 72, 153, 0.3)", text: "#ec4899" },
  };

  return (
    <>
      {/* DAG Statistics */}
      <section
        style={{
          marginBottom: "1.5rem",
          padding: "1.5rem",
          borderRadius: 16,
          background: "rgba(30, 41, 59, 0.7)",
          backdropFilter: "blur(12px)",
          border: "1px solid rgba(6, 182, 212, 0.2)",
          boxShadow: "0 8px 32px rgba(0, 0, 0, 0.3)",
        }}
      >
        <h2 style={{ fontSize: "1.4rem", marginBottom: "1rem", fontWeight: "600", color: "#f8fafc" }}>
          📊 DAG Statistics
        </h2>
        {loading && !dagStats ? (
          <StatCardSkeleton count={5} />
        ) : dagStats ? (
          <div style={{ display: "grid", gridTemplateColumns: "repeat(auto-fit, minmax(180px, 1fr))", gap: "1rem" }}>
            <div style={{
              padding: "1rem",
              borderRadius: 10,
              background: "rgba(99, 102, 241, 0.1)",
              border: "1px solid rgba(99, 102, 241, 0.2)"
            }}>
              <div style={{ fontSize: "2rem", fontWeight: "700", color: "#6366f1" }}>
                {dagStats.total_blocks}
              </div>
              <div style={{ color: "#94a3b8", fontSize: "0.9rem", marginTop: "0.25rem" }}>Total Blocks</div>
            </div>
            <div style={{
              padding: "1rem",
              borderRadius: 10,
              background: "rgba(6, 182, 212, 0.1)",
              border: "1px solid rgba(6, 182, 212, 0.2)"
            }}>
              <div style={{ fontSize: "2rem", fontWeight: "700", color: "#06b6d4" }}>
                {dagStats.blue_blocks}
              </div>
              <div style={{ color: "#94a3b8", fontSize: "0.9rem", marginTop: "0.25rem" }}>Blue Blocks</div>
            </div>
            <div style={{
              padding: "1rem",
              borderRadius: 10,
              background: "rgba(239, 68, 68, 0.1)",
              border: "1px solid rgba(239, 68, 68, 0.2)"
            }}>
              <div style={{ fontSize: "2rem", fontWeight: "700", color: "#ef4444" }}>
                {dagStats.red_blocks}
              </div>
              <div style={{ color: "#94a3b8", fontSize: "0.9rem", marginTop: "0.25rem" }}>Red Blocks</div>
            </div>
            <div style={{
              padding: "1rem",
              borderRadius: 10,
              background: "rgba(16, 185, 129, 0.1)",
              border: "1px solid rgba(16, 185, 129, 0.2)"
            }}>
              <div style={{ fontSize: "2rem", fontWeight: "700", color: "#10b981" }}>
                {dagStats.total_transactions}
              </div>
              <div style={{ color: "#94a3b8", fontSize: "0.9rem", marginTop: "0.25rem" }}>Total Transactions</div>
            </div>
            <div style={{
              padding: "1rem",
              borderRadius: 10,
              background: "rgba(139, 92, 246, 0.1)",
              border: "1px solid rgba(139, 92, 246, 0.2)"
            }}>
              <div style={{ fontSize: "2rem", fontWeight: "700", color: "#8b5cf6" }}>
                {dagStats.avg_txs_per_block.toFixed(2)}
              </div>
              <div style={{ color: "#94a3b8", fontSize: "0.9rem", marginTop: "0.25rem" }}>Avg Txs/Block</div>
            </div>
          </div>
        ) : (
          <p style={{ color: "#94a3b8", fontStyle: "italic" }}>Loading DAG stats...</p>
        )}
        <button
          onClick={refreshExplorer}
          disabled={loading}
          aria-label="Refresh explorer data"
          style={{
            marginTop: "1rem",
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

      {/* Blocks / DAG Visualization */}
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
        {/* Header with View Toggle */}
        <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: "1rem" }}>
          <h2 style={{ fontSize: "1.4rem", fontWeight: "600", color: "#f8fafc", margin: 0 }}>
            {dagViewMode === "graph" ? "🔗 DAG Visualization" : "📦 Recent Blocks"}
          </h2>
          <div style={{ display: "flex", gap: "0.5rem", alignItems: "center" }}>
            {dagViewMode === "graph" && (
              <>
                <button
                  onClick={() => setDagZoom(z => Math.max(0.5, z - 0.1))}
                  aria-label="Zoom out"
                  style={{
                    padding: "0.4rem 0.6rem",
                    borderRadius: 6,
                    border: "1px solid rgba(99, 102, 241, 0.3)",
                    background: "rgba(99, 102, 241, 0.1)",
                    color: "#6366f1",
                    cursor: "pointer",
                    fontSize: "0.9rem",
                  }}
                >
                  ➖
                </button>
                <span style={{ color: "#94a3b8", fontSize: "0.85rem", minWidth: "45px", textAlign: "center" }} aria-live="polite">
                  {Math.round(dagZoom * 100)}%
                </span>
                <button
                  onClick={() => setDagZoom(z => Math.min(2, z + 0.1))}
                  aria-label="Zoom in"
                  style={{
                    padding: "0.4rem 0.6rem",
                    borderRadius: 6,
                    border: "1px solid rgba(99, 102, 241, 0.3)",
                    background: "rgba(99, 102, 241, 0.1)",
                    color: "#6366f1",
                    cursor: "pointer",
                    fontSize: "0.9rem",
                  }}
                >
                  ➕
                </button>
                <button
                  onClick={() => { setDagZoom(1); setDagPan({ x: 0, y: 0 }); }}
                  aria-label="Reset zoom and pan"
                  style={{
                    padding: "0.4rem 0.6rem",
                    borderRadius: 6,
                    border: "1px solid rgba(99, 102, 241, 0.3)",
                    background: "rgba(99, 102, 241, 0.1)",
                    color: "#6366f1",
                    cursor: "pointer",
                    fontSize: "0.85rem",
                    marginRight: "0.5rem",
                  }}
                >
                  Reset
                </button>
              </>
            )}
            <button
              onClick={() => setDagViewMode(dagViewMode === "graph" ? "list" : "graph")}
              aria-label={dagViewMode === "graph" ? "Switch to list view" : "Switch to graph view"}
              style={{
                padding: "0.5rem 1rem",
                borderRadius: 8,
                border: "none",
                background: "linear-gradient(135deg, #6366f1, #4f46e5)",
                color: "white",
                cursor: "pointer",
                fontWeight: "600",
                fontSize: "0.85rem",
              }}
            >
              {dagViewMode === "graph" ? "📋 List View" : "🔗 Graph View"}
            </button>
          </div>
        </div>

        {/* DAG Graph View */}
        {dagViewMode === "graph" && blocks.length > 0 && (
          <div style={{ display: "flex", gap: "1rem" }}>
            <div
              style={{
                flex: 1,
                height: "500px",
                background: "rgba(2, 6, 23, 0.8)",
                borderRadius: 12,
                border: "1px solid rgba(99, 102, 241, 0.2)",
                overflow: "hidden",
                position: "relative",
              }}
            >
              <svg
                width="100%"
                height="100%"
                style={{ cursor: "grab" }}
                onMouseDown={(e) => {
                  const startX = e.clientX;
                  const startY = e.clientY;
                  const startPan = { ...dagPan };
                  const onMouseMove = (ev: MouseEvent) => {
                    setDagPan({
                      x: startPan.x + (ev.clientX - startX),
                      y: startPan.y + (ev.clientY - startY),
                    });
                  };
                  const onMouseUp = () => {
                    document.removeEventListener("mousemove", onMouseMove);
                    document.removeEventListener("mouseup", onMouseUp);
                  };
                  document.addEventListener("mousemove", onMouseMove);
                  document.addEventListener("mouseup", onMouseUp);
                }}
              >
                <defs>
                  <marker id="arrowhead" markerWidth="10" markerHeight="7" refX="9" refY="3.5" orient="auto">
                    <polygon points="0 0, 10 3.5, 0 7" fill="#64748b" />
                  </marker>
                  <linearGradient id="streamA" x1="0%" y1="0%" x2="100%" y2="100%">
                    <stop offset="0%" stopColor="#10b981" />
                    <stop offset="100%" stopColor="#059669" />
                  </linearGradient>
                  <linearGradient id="streamB" x1="0%" y1="0%" x2="100%" y2="100%">
                    <stop offset="0%" stopColor="#8b5cf6" />
                    <stop offset="100%" stopColor="#7c3aed" />
                  </linearGradient>
                  <linearGradient id="streamC" x1="0%" y1="0%" x2="100%" y2="100%">
                    <stop offset="0%" stopColor="#ec4899" />
                    <stop offset="100%" stopColor="#db2777" />
                  </linearGradient>
                </defs>
                <g transform={`translate(${dagPan.x}, ${dagPan.y}) scale(${dagZoom})`}>
                  {/* Draw edges */}
                  {blocks.map((block) => {
                    const blockNum = parseInt(block.number, 16);
                    const x = 100 + (blockNum % 10) * 120;
                    const y = 80 + Math.floor(blockNum / 10) * 100;
                    const parentHashes = block.parentHashes || (block.parentHash ? [block.parentHash] : []);

                    return parentHashes.map((parentHash, pIdx) => {
                      const parentBlock = blocks.find(b => b.hash === parentHash);
                      if (!parentBlock) return null;
                      const parentNum = parseInt(parentBlock.number, 16);
                      const px = 100 + (parentNum % 10) * 120;
                      const py = 80 + Math.floor(parentNum / 10) * 100;

                      return (
                        <line
                          key={`${block.hash}-${pIdx}`}
                          x1={px + 40}
                          y1={py + 20}
                          x2={x}
                          y2={y + 20}
                          stroke="#64748b"
                          strokeWidth="2"
                          strokeOpacity="0.6"
                          markerEnd="url(#arrowhead)"
                        />
                      );
                    });
                  })}

                  {/* Draw block nodes */}
                  {blocks.map((block) => {
                    const blockNum = parseInt(block.number, 16);
                    const x = 100 + (blockNum % 10) * 120;
                    const y = 80 + Math.floor(blockNum / 10) * 100;
                    const stream = block.streamType || block.stream_type || "A";
                    const isSelected = selectedBlock?.hash === block.hash;

                    return (
                      <g
                        key={block.hash}
                        onClick={() => setSelectedBlock(block)}
                        style={{ cursor: "pointer" }}
                        role="button"
                        tabIndex={0}
                        aria-label={`Block number ${blockNum}`}
                        onKeyDown={(e) => {
                          if (e.key === 'Enter' || e.key === ' ') {
                            e.preventDefault();
                            setSelectedBlock(block);
                          }
                        }}
                      >
                        <rect
                          x={x}
                          y={y}
                          width="80"
                          height="40"
                          rx="8"
                          fill={`url(#stream${stream})`}
                          stroke={isSelected ? "#f8fafc" : "transparent"}
                          strokeWidth={isSelected ? 3 : 0}
                        />
                        <text
                          x={x + 40}
                          y={y + 18}
                          fill="white"
                          fontSize="12"
                          fontWeight="bold"
                          textAnchor="middle"
                        >
                          #{blockNum}
                        </text>
                        <text
                          x={x + 40}
                          y={y + 32}
                          fill="rgba(255,255,255,0.8)"
                          fontSize="10"
                          textAnchor="middle"
                        >
                          {block.transactions.length} tx
                        </text>
                      </g>
                    );
                  })}
                </g>
              </svg>
            </div>

            {/* Block Detail Panel */}
            {selectedBlock && (
              <div style={{
                width: "320px",
                background: "rgba(2, 6, 23, 0.8)",
                borderRadius: 12,
                border: "1px solid rgba(99, 102, 241, 0.3)",
                padding: "1rem",
                overflowY: "auto",
                maxHeight: "500px",
              }}>
                <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: "1rem" }}>
                  <h3 style={{ color: "#f8fafc", fontSize: "1.1rem", margin: 0 }}>Block Details</h3>
                  <button
                    onClick={() => setSelectedBlock(null)}
                    aria-label="Close block details"
                    style={{
                      background: "transparent",
                      border: "none",
                      color: "#94a3b8",
                      cursor: "pointer",
                      fontSize: "1.2rem",
                    }}
                  >
                    ✕
                  </button>
                </div>
                <div style={{ display: "flex", flexDirection: "column", gap: "0.75rem" }}>
                  <div>
                    <div style={{ color: "#64748b", fontSize: "0.75rem", marginBottom: "0.25rem" }}>BLOCK NUMBER</div>
                    <div style={{ color: "#f8fafc", fontWeight: "600", fontSize: "1.2rem" }}>
                      #{parseInt(selectedBlock.number, 16)}
                    </div>
                  </div>
                  <div>
                    <div style={{ color: "#64748b", fontSize: "0.75rem", marginBottom: "0.25rem" }}>HASH</div>
                    <div style={{
                      color: "#06b6d4",
                      fontFamily: "'JetBrains Mono', monospace",
                      fontSize: "0.75rem",
                      wordBreak: "break-all",
                    }}>
                      {selectedBlock.hash}
                    </div>
                  </div>
                  <div>
                    <div style={{ color: "#64748b", fontSize: "0.75rem", marginBottom: "0.25rem" }}>TRANSACTIONS</div>
                    <div style={{ color: "#10b981", fontWeight: "600" }}>
                      {selectedBlock.transactions.length}
                    </div>
                  </div>
                </div>
              </div>
            )}
          </div>
        )}

        {dagViewMode === "graph" && blocks.length === 0 && (
          <div style={{
            height: "300px",
            display: "flex",
            alignItems: "center",
            justifyContent: "center",
            background: "rgba(2, 6, 23, 0.5)",
            borderRadius: 12,
          }}>
            <p style={{ color: "#94a3b8", fontStyle: "italic" }}>No blocks to visualize. Start mining to see the DAG.</p>
          </div>
        )}

        {/* List View */}
        {dagViewMode === "list" && blocks.length > 0 && (
          <div style={{ overflowX: "auto", overflowY: "auto", maxHeight: "600px" }}>
            {blocks.map((block) => {
              const stream = (block.streamType || block.stream_type || 'A') as string;
              const colors = streamColors[stream] ?? streamColors['A'];
              void (block.parentHashes || block.parentHash);

              return (
                <div
                  key={block.hash}
                  onClick={() => { setSelectedBlock(block); setDagViewMode("graph"); }}
                  role="button"
                  tabIndex={0}
                  aria-label={`Block number ${parseInt(block.number, 16)}`}
                  onKeyDown={(e) => {
                    if (e.key === 'Enter' || e.key === ' ') {
                      e.preventDefault();
                      setSelectedBlock(block);
                      setDagViewMode("graph");
                    }
                  }}
                  style={{
                    padding: "1rem",
                    marginBottom: "0.75rem",
                    borderRadius: 10,
                    background: colors.bg,
                    border: `1px solid ${colors.border}`,
                    fontSize: "0.95rem",
                    cursor: "pointer",
                  }}
                >
                  <div style={{ marginBottom: "0.5rem", display: "flex", justifyContent: "space-between", alignItems: "center" }}>
                    <div>
                      <strong style={{ fontSize: "1.1rem", color: "#f8fafc" }}>
                        Block #{parseInt(block.number, 16)}
                      </strong>
                      <span style={{
                        marginLeft: "0.75rem",
                        padding: "0.25rem 0.75rem",
                        background: colors.border,
                        borderRadius: 6,
                        fontSize: "0.85rem",
                        fontWeight: "600",
                        color: colors.text
                      }}>
                        Stream {stream}
                      </span>
                    </div>
                  </div>
                  <div style={{ color: "#94a3b8", fontSize: "0.85rem" }}>
                    <strong>Transactions:</strong> {block.transactions.length}
                  </div>
                </div>
              );
            })}
          </div>
        )}
      </section>
    </>
  );
};
