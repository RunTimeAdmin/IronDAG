import React, { useState, useEffect, useRef, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import logoHero from "../assets/logo-hero.png?url";
import type { TabType, NodeStatus, UpdateInfo } from "../types";
import { ErrorBoundary, ConfirmDialog, ToastContainer } from "./common";
import { Dashboard } from "./Dashboard";
import { Wallet } from "./Wallet";
import { SendTransaction } from "./SendTransaction";
import { TransactionHistory } from "./TransactionHistory";
import { Explorer } from "./Explorer";
import { Metrics } from "./Metrics";
import { AccountAbstraction } from "./AccountAbstraction";
import { AdvancedFeatures } from "./AdvancedFeatures";
import { Settings } from "./Settings";
import { useToast } from "../hooks/useToast";

const tabs: { id: TabType; label: string; color: string }[] = [
  { id: "dashboard", label: "Dashboard", color: "#6366f1" },
  { id: "wallet", label: "Wallet", color: "#6366f1" },
  { id: "send", label: "Send", color: "#6366f1" },
  { id: "history", label: "History", color: "#6366f1" },
  { id: "explorer", label: "Explorer", color: "#6366f1" },
  { id: "metrics", label: "Metrics", color: "#6366f1" },
  { id: "account-abstraction", label: "Account Abstraction", color: "#8b5cf6" },
  { id: "privacy", label: "Privacy", color: "#ec4899" },
  { id: "oracles", label: "Oracles", color: "#06b6d4" },
  { id: "recurring", label: "Recurring", color: "#10b981" },
  { id: "stop-loss", label: "Stop-Loss", color: "#f59e0b" },
  { id: "settings", label: "Settings", color: "#64748b" },
];

export const Layout: React.FC = () => {
  const [activeTab, setActiveTabState] = useState<TabType>("dashboard");
  const [nodeStatus, setNodeStatus] = useState<NodeStatus | null>(null);
  const [isRefreshing, setIsRefreshing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [updateAvailable, setUpdateAvailable] = useState<UpdateInfo>(null);
  const [updateProgress, setUpdateProgress] = useState<number | null>(null);
  const [confirmDialog, setConfirmDialog] = useState<{
    title: string;
    message: string;
    onConfirm: () => void;
  } | null>(null);
  
  // Toast system
  const { toasts, addToast, removeToast } = useToast();
  
  // Connection state tracking
  const [_connectionState, setConnectionState] = useState<"connected" | "disconnected" | "reconnecting">("disconnected");
  const [showReconnectBanner, setShowReconnectBanner] = useState(false);
  const prevNodeStatus = useRef<NodeStatus | null>(null);

  // Load active tab from settings on mount
  useEffect(() => {
    invoke<{ active_tab: string }>("get_settings")
      .then((settings) => {
        if (settings.active_tab && tabs.some(t => t.id === settings.active_tab)) {
          setActiveTabState(settings.active_tab as TabType);
        }
      })
      .catch(() => {});
  }, []);

  // Wrapper to persist tab changes
  const setActiveTab = async (tab: TabType) => {
    setActiveTabState(tab);
    try {
      await invoke("update_setting", { key: "active_tab", value: tab });
    } catch (e) {
      console.error("Failed to persist active tab:", e);
    }
  };

  // Window resize listener for size persistence
  useEffect(() => {
    let timeout: ReturnType<typeof setTimeout>;
    const handleResize = () => {
      clearTimeout(timeout);
      timeout = setTimeout(() => {
        invoke("update_setting", { key: "window_width", value: String(window.innerWidth) }).catch(() => {});
        invoke("update_setting", { key: "window_height", value: String(window.innerHeight) }).catch(() => {});
      }, 500); // Debounce 500ms
    };
    window.addEventListener("resize", handleResize);
    return () => {
      window.removeEventListener("resize", handleResize);
      clearTimeout(timeout);
    };
  }, []);
  const mainContentRef = useRef<HTMLElement>(null);
  const tablistRef = useRef<HTMLDivElement>(null);

  // Handle keyboard navigation for tabs
  const handleTabKeyDown = (e: React.KeyboardEvent, currentIndex: number) => {
    let newIndex = currentIndex;
    
    switch (e.key) {
      case "ArrowLeft":
        e.preventDefault();
        newIndex = currentIndex > 0 ? currentIndex - 1 : tabs.length - 1;
        setActiveTab(tabs[newIndex].id);
        break;
      case "ArrowRight":
        e.preventDefault();
        newIndex = currentIndex < tabs.length - 1 ? currentIndex + 1 : 0;
        setActiveTab(tabs[newIndex].id);
        break;
      case "Home":
        e.preventDefault();
        setActiveTab(tabs[0].id);
        break;
      case "End":
        e.preventDefault();
        setActiveTab(tabs[tabs.length - 1].id);
        break;
    }
    
    // Focus the new tab button
    const tabButtons = tablistRef.current?.querySelectorAll('[role="tab"]');
    if (tabButtons && tabButtons[newIndex]) {
      (tabButtons[newIndex] as HTMLElement).focus();
    }
  };

  // Focus first focusable element when tab changes
  useEffect(() => {
    if (mainContentRef.current) {
      const focusable = mainContentRef.current.querySelector<HTMLElement>(
        'button, [href], input, select, textarea, [tabindex]:not([tabindex="-1"])'
      );
      if (focusable) {
        focusable.focus();
      }
    }
  }, [activeTab]);

  // Auto-update check
  useEffect(() => {
    const checkUpdate = async () => {
      try {
        const { check } = await import("@tauri-apps/plugin-updater");
        const update = await check();
        if (update) {
          setUpdateAvailable({ version: update.version, body: update.body || "" });
        }
      } catch (e) {
        console.log("Update check skipped:", e);
      }
    };
    const timer = setTimeout(checkUpdate, 5000);
    return () => clearTimeout(timer);
  }, []);

  const installUpdate = async () => {
    try {
      const { check } = await import("@tauri-apps/plugin-updater");
      const update = await check();
      if (update) {
        setUpdateProgress(0);
        await update.downloadAndInstall((event) => {
          if (event.event === "Progress") {
            const progress = event.data.chunkLength;
            setUpdateProgress((prev) => (prev || 0) + progress);
          }
        });
        const { relaunch } = await import("@tauri-apps/plugin-process");
        await relaunch();
      }
    } catch (e) {
      console.error("Update failed:", e);
      setUpdateProgress(null);
    }
  };

  // Refresh node status
  const refresh = useCallback(async () => {
    if (isRefreshing) return;
    setIsRefreshing(true);
    try {
      const node = await invoke<NodeStatus>("get_node_status");
      setNodeStatus(node);
      setError(null);
      
      // Handle connection state changes
      if (prevNodeStatus.current === null && node) {
        // Initial connection
        setConnectionState("connected");
        addToast("Connected to node", "success");
      } else if (prevNodeStatus.current !== null && !node) {
        // Connection lost
        setConnectionState("disconnected");
        setShowReconnectBanner(true);
      } else if (prevNodeStatus.current === null && !node) {
        // Still no connection
        setConnectionState("disconnected");
      } else if (showReconnectBanner && node) {
        // Reconnected
        setConnectionState("connected");
        setShowReconnectBanner(false);
        addToast("Node reconnected", "success");
      }
      prevNodeStatus.current = node;
    } catch (e: any) {
      if (nodeStatus !== null) {
        setError("Node connection lost");
        setConnectionState("disconnected");
        setShowReconnectBanner(true);
      }
      setNodeStatus(null);
      prevNodeStatus.current = null;
    } finally {
      setIsRefreshing(false);
    }
  }, [isRefreshing, nodeStatus, addToast, showReconnectBanner]);

  // Initial refresh and polling
  useEffect(() => {
    refresh().catch(() => {});
    const interval = setInterval(refresh, 10000);
    return () => clearInterval(interval);
  }, []);

  const renderTabContent = () => {
    switch (activeTab) {
      case "dashboard":
        return (
          <Dashboard
            nodeStatus={nodeStatus}
            isRefreshing={isRefreshing}
            onRefresh={refresh}
            setError={setError}
            setConfirmDialog={setConfirmDialog}
            addToast={addToast}
          />
        );
      case "wallet":
        return <Wallet setError={setError} setConfirmDialog={setConfirmDialog} addToast={addToast} />;
      case "send":
        return <SendTransaction setError={setError} setConfirmDialog={setConfirmDialog} />;
      case "history":
        return <TransactionHistory setError={setError} />;
      case "explorer":
        return <Explorer setError={setError} />;
      case "metrics":
        return <Metrics setError={setError} />;
      case "account-abstraction":
        return <AccountAbstraction setError={setError} setConfirmDialog={setConfirmDialog} />;
      case "privacy":
      case "oracles":
      case "recurring":
      case "stop-loss":
        return <AdvancedFeatures activeTab={activeTab} setError={setError} setConfirmDialog={setConfirmDialog} />;
      case "settings":
        return <Settings setError={setError} />;
      default:
        return <Dashboard nodeStatus={nodeStatus} isRefreshing={isRefreshing} onRefresh={refresh} setError={setError} setConfirmDialog={setConfirmDialog} addToast={addToast} />;
    }
  };

  return (
    <div
      style={{
        minHeight: "100vh",
        padding: "2rem",
        fontFamily: "'Segoe UI', system-ui, -apple-system, BlinkMacSystemFont, sans-serif",
        background: "linear-gradient(135deg, #020617 0%, #0f172a 50%, #1e293b 100%)",
        color: "#f8fafc",
      }}
    >
      <div style={{ maxWidth: "1400px", margin: "0 auto" }}>
        {/* Skip to main content link */}
        <a
          href="#main-content"
          className="skip-link"
          style={{
            position: "absolute",
            left: "-9999px",
            top: 0,
            zIndex: 9999,
            padding: "8px 16px",
            background: "#6366f1",
            color: "white",
            textDecoration: "none",
            borderRadius: "0 0 4px 0",
          }}
          onFocus={(e) => {
            e.currentTarget.style.left = "0";
          }}
          onBlur={(e) => {
            e.currentTarget.style.left = "-9999px";
          }}
        >
          Skip to main content
        </a>

        {/* Update Banner */}
        {updateAvailable && (
          <div className="update-banner" role="alert" aria-live="polite">
            <span>Update v{updateAvailable.version} available</span>
            {updateProgress !== null ? (
              <span>Downloading...</span>
            ) : (
              <button onClick={installUpdate} aria-label="Install update now">Install Now</button>
            )}
            <button 
              className="dismiss" 
              onClick={() => setUpdateAvailable(null)}
              aria-label="Dismiss update notification"
            >
              ×
            </button>
          </div>
        )}

        {/* Reconnection Banner */}
        {showReconnectBanner && (
          <div
            role="alert"
            aria-live="polite"
            style={{
              background: "rgba(245, 158, 11, 0.15)",
              border: "1px solid rgba(245, 158, 11, 0.4)",
              padding: "0.875rem 1.25rem",
              borderRadius: 12,
              marginBottom: "1rem",
              display: "flex",
              alignItems: "center",
              justifyContent: "center",
              gap: "0.75rem",
              backdropFilter: "blur(12px)"
            }}
          >
            <div
              style={{
                width: "10px",
                height: "10px",
                borderRadius: "50%",
                background: "#f59e0b",
                animation: "pulse 1.5s ease-in-out infinite"
              }}
            />
            <span style={{ color: "#fbbf24", fontWeight: "500" }}>
              Node disconnected. Attempting to reconnect...
            </span>
          </div>
        )}

        {/* Header */}
        <div style={{ marginBottom: "2rem", textAlign: "center" }}>
          <img
            src={logoHero}
            alt="IronDAG Logo"
            style={{
              width: "200px",
              height: "200px",
              objectFit: "contain",
              marginBottom: "1rem",
              filter: "drop-shadow(0 0 30px rgba(99, 102, 241, 0.5))",
              animation: "pulse 3s ease-in-out infinite",
              display: "block",
              margin: "0 auto 1rem auto"
            }}
          />
          <h1
            style={{
              fontSize: "2.5rem",
              marginBottom: "0.5rem",
              background: "linear-gradient(135deg, #6366f1, #ec4899, #06b6d4)",
              WebkitBackgroundClip: "text",
              WebkitTextFillColor: "transparent",
              backgroundClip: "text",
              fontWeight: "700",
              letterSpacing: "-0.02em"
            }}
          >
            IronDAG Desktop
          </h1>
          <p
            style={{
              opacity: 0.8,
              marginBottom: 0,
              fontSize: "1.1rem",
              color: "#94a3b8"
            }}
          >
            All-in-One Blockchain Experience
          </p>
        </div>

        {/* Connection Status */}
        <div
          style={{
            marginBottom: "1.5rem",
            padding: "0.75rem 1.5rem",
            borderRadius: 12,
            background: nodeStatus
              ? "rgba(16, 185, 129, 0.1)"
              : "rgba(239, 68, 68, 0.1)",
            border: nodeStatus
              ? "1px solid rgba(16, 185, 129, 0.3)"
              : "1px solid rgba(239, 68, 68, 0.3)",
            display: "flex",
            alignItems: "center",
            justifyContent: "center",
            gap: "0.75rem",
            backdropFilter: "blur(12px)"
          }}
          aria-live="polite"
          aria-label={nodeStatus ? `Node status: connected at height ${nodeStatus.height}` : "Node status: not connected"}
        >
          <div
            style={{
              width: "10px",
              height: "10px",
              borderRadius: "50%",
              background: nodeStatus ? "#10b981" : "#ef4444",
              boxShadow: nodeStatus
                ? "0 0 10px rgba(16, 185, 129, 0.6)"
                : "0 0 10px rgba(239, 68, 68, 0.6)",
              animation: nodeStatus ? "pulse 2s ease-in-out infinite" : "none"
            }}
            aria-hidden="true"
          />
          <span
            style={{
              color: nodeStatus ? "#34d399" : "#fca5a5",
              fontWeight: "600",
              fontSize: "0.95rem"
            }}
          >
            {nodeStatus
              ? `Connected to Node (Height: ${nodeStatus.height})`
              : "Not Connected - Start a node from Dashboard"}
          </span>
        </div>

        {/* Tab Navigation */}
        <div
          ref={tablistRef}
          role="tablist"
          aria-label="Main navigation"
          style={{
            marginBottom: "2rem",
            display: "flex",
            gap: "0.75rem",
            justifyContent: "center",
            flexWrap: "wrap"
          }}
        >
          {tabs.map((tab, index) => (
            <button
              key={tab.id}
              role="tab"
              aria-selected={activeTab === tab.id}
              aria-label={`Switch to ${tab.label} tab`}
              tabIndex={activeTab === tab.id ? 0 : -1}
              onClick={() => setActiveTab(tab.id)}
              onKeyDown={(e) => handleTabKeyDown(e, index)}
              style={{
                padding: "0.75rem 1.5rem",
                borderRadius: 8,
                border: "none",
                cursor: "pointer",
                background:
                  activeTab === tab.id
                    ? `linear-gradient(135deg, ${tab.color}, ${tab.color}dd)`
                    : "rgba(30, 41, 59, 0.7)",
                color: "#f8fafc",
                fontWeight: "600",
                fontSize: "0.95rem",
                boxShadow:
                  activeTab === tab.id
                    ? `0 4px 12px ${tab.color}4d`
                    : "none",
                transition: "all 0.3s ease",
                backdropFilter: "blur(12px)"
              }}
            >
              {tab.label}
            </button>
          ))}
        </div>

        {/* Error Display */}
        {error && (
          <div
            role="alert"
            aria-live="assertive"
            style={{
              background: "rgba(239, 68, 68, 0.1)",
              border: "1px solid rgba(239, 68, 68, 0.3)",
              padding: "1rem 1.25rem",
              borderRadius: 12,
              marginBottom: "1.5rem",
              backdropFilter: "blur(12px)",
              boxShadow: "0 4px 12px rgba(239, 68, 68, 0.1)"
            }}
          >
            <strong style={{ color: "#fca5a5" }}>Error:</strong>{" "}
            <span style={{ color: "#fecaca" }}>{error}</span>
          </div>
        )}

        {/* Tab Content */}
        <main
          id="main-content"
          role="tabpanel"
          ref={mainContentRef}
          aria-label={`${tabs.find(t => t.id === activeTab)?.label || 'Current'} tab content`}
        >
          <ErrorBoundary key={activeTab}>
            {renderTabContent()}
          </ErrorBoundary>
        </main>

        {/* Confirmation Dialog */}
        <ConfirmDialog
          title={confirmDialog?.title || ""}
          message={confirmDialog?.message || ""}
          isOpen={!!confirmDialog}
          onConfirm={() => {
            confirmDialog?.onConfirm();
            setConfirmDialog(null);
          }}
          onCancel={() => setConfirmDialog(null)}
        />
      </div>
      
      {/* Toast Container */}
      <ToastContainer toasts={toasts} onDismiss={removeToast} />
    </div>
  );
};
