import React, { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useKeystore } from "../hooks/useKeystore";
import { ConfirmDialog } from "./common/ConfirmDialog";

interface SettingsProps {
  setError: (error: string | null) => void;
}

interface AppSettings {
  rpc_url: string;
  active_tab: string;
  window_width: number;
  window_height: number;
  auto_start_node: boolean;
  log_level: string;
  theme: string;
}

export const Settings: React.FC<SettingsProps> = ({ setError }) => {
  const [rpcUrl, setRpcUrl] = useState<string>("http://127.0.0.1:8546");
  const [loading, setLoading] = useState(false);
  const [showDeleteConfirm, setShowDeleteConfirm] = useState(false);
  const [showResetConfirm, setShowResetConfirm] = useState(false);
  const [password, setPassword] = useState("");
  const [newPassword, setNewPassword] = useState("");
  const [showChangePassword, setShowChangePassword] = useState(false);
  const [settings, setSettings] = useState<AppSettings | null>(null);
  const [dataDir] = useState<string>("");

  const {
    hasKeystore,
    isUnlocked,
    walletAddress,
    deleteKeystore,
  } = useKeystore();

  // Load settings on mount
  useEffect(() => {
    invoke<AppSettings>("get_settings")
      .then((s) => {
        setSettings(s);
        setRpcUrl(s.rpc_url);
      })
      .catch(() => {});
    
    // Get data directory
    invoke<string>("get_rpc_url").catch(() => {});
  }, []);

  const updateRpcUrl = async (newUrl: string) => {
    setRpcUrl(newUrl);
  };

  const saveRpcUrl = async () => {
    setLoading(true);
    try {
      await invoke("set_rpc_url", { newUrl: rpcUrl });
      await invoke("update_setting", { key: "rpc_url", value: rpcUrl });
      setError(null);
      alert("RPC URL saved!");
    } catch (e: any) {
      setError(e?.toString?.() ?? "Failed to save RPC URL");
    } finally {
      setLoading(false);
    }
  };

  const handleTestConnection = async () => {
    setLoading(true);
    try {
      await invoke("set_rpc_url", { newUrl: rpcUrl });
      await invoke("get_node_status");
      setError(null);
      alert("Connection successful!");
    } catch (e: any) {
      setError(e?.toString?.() ?? "Connection failed");
    } finally {
      setLoading(false);
    }
  };

  const handleDeleteKeystore = async () => {
    try {
      await deleteKeystore();
      setShowDeleteConfirm(false);
      setError(null);
    } catch (e: any) {
      setError(e?.toString?.() ?? "Failed to delete keystore");
    }
  };

  const handleChangePassword = async () => {
    if (!password || !newPassword) {
      setError("Both current and new password are required");
      return;
    }
    setLoading(true);
    try {
      await invoke("change_keystore_password", {
        currentPassword: password,
        newPassword: newPassword,
      });
      setPassword("");
      setNewPassword("");
      setShowChangePassword(false);
      setError(null);
      alert("Password changed successfully!");
    } catch (e: any) {
      setError(e?.toString?.() ?? "Failed to change password");
    } finally {
      setLoading(false);
    }
  };

  const handleSettingChange = async (key: string, value: string) => {
    try {
      const updated = await invoke<AppSettings>("update_setting", { key, value });
      setSettings(updated);
      setError(null);
    } catch (e: any) {
      setError(e?.toString?.() ?? `Failed to update ${key}`);
    }
  };

  const handleResetSettings = async () => {
    setLoading(true);
    try {
      const defaults = await invoke<AppSettings>("reset_settings");
      setSettings(defaults);
      setRpcUrl(defaults.rpc_url);
      await invoke("set_rpc_url", { newUrl: defaults.rpc_url });
      setShowResetConfirm(false);
      setError(null);
      alert("Settings reset to defaults!");
    } catch (e: any) {
      setError(e?.toString?.() ?? "Failed to reset settings");
    } finally {
      setLoading(false);
    }
  };

  const getKeystoreStatus = () => {
    if (!hasKeystore) return { text: "Not Created", color: "#94a3b8" };
    if (isUnlocked) return { text: "Unlocked", color: "#10b981" };
    return { text: "Locked", color: "#f59e0b" };
  };

  const keystoreStatus = getKeystoreStatus();

  return (
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
      <h2
        style={{
          fontSize: "1.4rem",
          marginBottom: "1rem",
          fontWeight: "600",
          color: "#f8fafc",
        }}
      >
        ⚙️ Settings
      </h2>

      {/* Connection Section */}
      <div
        style={{
          marginBottom: "1.5rem",
          padding: "1rem",
          borderRadius: 12,
          background: "rgba(2, 6, 23, 0.4)",
        }}
      >
        <h3
          style={{
            fontSize: "1.1rem",
            marginBottom: "0.75rem",
            color: "#e2e8f0",
          }}
        >
          🔗 Connection
        </h3>
        <label
          style={{
            display: "block",
            marginBottom: "0.5rem",
            color: "#94a3b8",
            fontWeight: "500",
          }}
        >
          RPC Endpoint
        </label>
        <div style={{ display: "flex", gap: "0.5rem", flexWrap: "wrap" }}>
          <input
            type="text"
            value={rpcUrl}
            onChange={(e) => updateRpcUrl(e.target.value)}
            placeholder="http://127.0.0.1:8546"
            aria-label="RPC endpoint URL"
            style={{
              flex: 1,
              minWidth: "200px",
              padding: "0.6rem 0.8rem",
              borderRadius: 8,
              border: "1px solid rgba(99, 102, 241, 0.3)",
              background: "rgba(2, 6, 23, 0.6)",
              color: "#e5e7eb",
              fontSize: "0.95rem",
            }}
          />
          <button
            onClick={saveRpcUrl}
            disabled={loading}
            aria-label="Save RPC URL"
            style={{
              padding: "0.6rem 1rem",
              borderRadius: 8,
              border: "none",
              background: loading
                ? "rgba(99, 102, 241, 0.5)"
                : "linear-gradient(135deg, #10b981, #059669)",
              color: "white",
              cursor: loading ? "not-allowed" : "pointer",
              fontWeight: "600",
            }}
          >
            Save
          </button>
          <button
            onClick={handleTestConnection}
            disabled={loading}
            aria-label="Test RPC connection"
            style={{
              padding: "0.6rem 1rem",
              borderRadius: 8,
              border: "none",
              background: loading
                ? "rgba(99, 102, 241, 0.5)"
                : "linear-gradient(135deg, #6366f1, #4f46e5)",
              color: "white",
              cursor: loading ? "not-allowed" : "pointer",
              fontWeight: "600",
            }}
          >
            {loading ? "⏳ Testing..." : "Test Connection"}
          </button>
        </div>
      </div>

      {/* Startup Section */}
      <div
        style={{
          marginBottom: "1.5rem",
          padding: "1rem",
          borderRadius: 12,
          background: "rgba(2, 6, 23, 0.4)",
        }}
      >
        <h3
          style={{
            fontSize: "1.1rem",
            marginBottom: "0.75rem",
            color: "#e2e8f0",
          }}
        >
          🚀 Startup
        </h3>
        
        <div style={{ marginBottom: "1rem" }}>
          <label
            style={{
              display: "flex",
              alignItems: "center",
              gap: "0.75rem",
              cursor: "pointer",
            }}
          >
            <input
              type="checkbox"
              checked={settings?.auto_start_node ?? false}
              onChange={(e) =>
                handleSettingChange("auto_start_node", String(e.target.checked))
              }
              aria-label="Auto-start node on app launch"
              style={{ width: 18, height: 18, cursor: "pointer" }}
            />
            <span style={{ color: "#e2e8f0" }}>
              Auto-start node on app launch
            </span>
          </label>
        </div>

        <div>
          <label
            style={{
              display: "block",
              marginBottom: "0.5rem",
              color: "#94a3b8",
              fontWeight: "500",
            }}
          >
            Log Level
          </label>
          <select
            value={settings?.log_level ?? "info"}
            onChange={(e) => handleSettingChange("log_level", e.target.value)}
            aria-label="Log level"
            style={{
              padding: "0.6rem 0.8rem",
              borderRadius: 8,
              border: "1px solid rgba(99, 102, 241, 0.3)",
              background: "rgba(2, 6, 23, 0.6)",
              color: "#e5e7eb",
              fontSize: "0.95rem",
              minWidth: "150px",
              cursor: "pointer",
            }}
          >
            <option value="debug">Debug</option>
            <option value="info">Info</option>
            <option value="warn">Warn</option>
            <option value="error">Error</option>
          </select>
        </div>
      </div>

      {/* Appearance Section */}
      <div
        style={{
          marginBottom: "1.5rem",
          padding: "1rem",
          borderRadius: 12,
          background: "rgba(2, 6, 23, 0.4)",
        }}
      >
        <h3
          style={{
            fontSize: "1.1rem",
            marginBottom: "0.75rem",
            color: "#e2e8f0",
          }}
        >
          🎨 Appearance
        </h3>
        
        <label
          style={{
            display: "block",
            marginBottom: "0.5rem",
            color: "#94a3b8",
            fontWeight: "500",
          }}
        >
          Theme
        </label>
        <div style={{ display: "flex", gap: "0.5rem" }}>
          <button
            disabled
            style={{
              padding: "0.6rem 1rem",
              borderRadius: 8,
              border: "2px solid rgba(99, 102, 241, 0.5)",
              background: "rgba(99, 102, 241, 0.2)",
              color: "#e2e8f0",
              fontWeight: "600",
              cursor: "default",
            }}
          >
            🌙 Dark (Active)
          </button>
          <button
            disabled
            style={{
              padding: "0.6rem 1rem",
              borderRadius: 8,
              border: "1px solid rgba(148, 163, 184, 0.3)",
              background: "transparent",
              color: "#64748b",
              fontWeight: "600",
              cursor: "not-allowed",
              opacity: 0.5,
            }}
          >
            ☀️ Light (Coming Soon)
          </button>
        </div>
      </div>

      {/* Keystore Section */}
      <div
        style={{
          marginBottom: "1.5rem",
          padding: "1rem",
          borderRadius: 12,
          background: "rgba(2, 6, 23, 0.4)",
        }}
      >
        <h3
          style={{
            fontSize: "1.1rem",
            marginBottom: "0.75rem",
            color: "#e2e8f0",
          }}
        >
          🔐 Keystore
        </h3>

        {/* Keystore Status */}
        <div
          style={{
            marginBottom: "1rem",
            padding: "0.75rem",
            borderRadius: 8,
            background: "rgba(30, 41, 59, 0.5)",
            display: "flex",
            alignItems: "center",
            justifyContent: "space-between",
          }}
        >
          <span style={{ color: "#94a3b8" }}>Status:</span>
          <span
            style={{
              color: keystoreStatus.color,
              fontWeight: "600",
            }}
          >
            {keystoreStatus.text}
          </span>
        </div>

        {isUnlocked && walletAddress && (
          <div
            style={{
              marginBottom: "1rem",
              padding: "0.75rem",
              borderRadius: 8,
              background: "rgba(16, 185, 129, 0.1)",
              border: "1px solid rgba(16, 185, 129, 0.3)",
            }}
          >
            <strong style={{ color: "#94a3b8" }}>Wallet Address:</strong>
            <div style={{ fontFamily: "monospace", color: "#e2e8f0" }}>
              {walletAddress}
            </div>
          </div>
        )}

        {/* Change Password Section */}
        {hasKeystore && !showChangePassword && (
          <button
            onClick={() => setShowChangePassword(true)}
            aria-label="Change keystore password"
            style={{
              padding: "0.6rem 1rem",
              borderRadius: 8,
              border: "none",
              background: "linear-gradient(135deg, #6366f1, #4f46e5)",
              color: "white",
              cursor: "pointer",
              fontWeight: "600",
              marginRight: "0.5rem",
              marginBottom: "0.5rem",
            }}
          >
            Change Password
          </button>
        )}

        {showChangePassword && (
          <div
            style={{
              marginBottom: "1rem",
              padding: "1rem",
              borderRadius: 8,
              background: "rgba(30, 41, 59, 0.5)",
            }}
          >
            <input
              type="password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              placeholder="Current password"
              aria-label="Current password"
              style={{
                width: "100%",
                padding: "0.6rem 0.8rem",
                borderRadius: 8,
                border: "1px solid rgba(99, 102, 241, 0.3)",
                background: "rgba(2, 6, 23, 0.6)",
                color: "#e5e7eb",
                marginBottom: "0.5rem",
                boxSizing: "border-box",
              }}
            />
            <input
              type="password"
              value={newPassword}
              onChange={(e) => setNewPassword(e.target.value)}
              placeholder="New password"
              aria-label="New password"
              style={{
                width: "100%",
                padding: "0.6rem 0.8rem",
                borderRadius: 8,
                border: "1px solid rgba(99, 102, 241, 0.3)",
                background: "rgba(2, 6, 23, 0.6)",
                color: "#e5e7eb",
                marginBottom: "0.5rem",
                boxSizing: "border-box",
              }}
            />
            <div style={{ display: "flex", gap: "0.5rem" }}>
              <button
                onClick={handleChangePassword}
                disabled={loading}
                aria-label="Save new password"
                style={{
                  padding: "0.6rem 1rem",
                  borderRadius: 8,
                  border: "none",
                  background: loading
                    ? "rgba(99, 102, 241, 0.5)"
                    : "linear-gradient(135deg, #6366f1, #4f46e5)",
                  color: "white",
                  cursor: loading ? "not-allowed" : "pointer",
                  fontWeight: "600",
                }}
              >
                {loading ? "⏳ Changing..." : "Save"}
              </button>
              <button
                onClick={() => {
                  setShowChangePassword(false);
                  setPassword("");
                  setNewPassword("");
                }}
                aria-label="Cancel password change"
                style={{
                  padding: "0.6rem 1rem",
                  borderRadius: 8,
                  border: "1px solid rgba(148, 163, 184, 0.3)",
                  background: "transparent",
                  color: "#94a3b8",
                  cursor: "pointer",
                }}
              >
                Cancel
              </button>
            </div>
          </div>
        )}

        {/* Delete Keystore Button */}
        {hasKeystore && (
          <button
            onClick={() => setShowDeleteConfirm(true)}
            aria-label="Delete keystore"
            style={{
              padding: "0.6rem 1rem",
              borderRadius: 8,
              border: "none",
              background: "linear-gradient(135deg, #ef4444, #dc2626)",
              color: "white",
              cursor: "pointer",
              fontWeight: "600",
            }}
          >
            🗑️ Delete Keystore
          </button>
        )}
      </div>

      {/* Data Section */}
      <div
        style={{
          marginBottom: "1.5rem",
          padding: "1rem",
          borderRadius: 12,
          background: "rgba(2, 6, 23, 0.4)",
        }}
      >
        <h3
          style={{
            fontSize: "1.1rem",
            marginBottom: "0.75rem",
            color: "#e2e8f0",
          }}
        >
          📁 Data
        </h3>
        
        <div
          style={{
            marginBottom: "1rem",
            padding: "0.75rem",
            borderRadius: 8,
            background: "rgba(30, 41, 59, 0.5)",
          }}
        >
          <strong style={{ color: "#94a3b8", display: "block", marginBottom: "0.25rem" }}>
            Data Directory:
          </strong>
          <code
            style={{
              color: "#64748b",
              fontSize: "0.85rem",
              wordBreak: "break-all",
            }}
          >
            {dataDir || "%APPDATA%\\irondag"}
          </code>
        </div>

        <div style={{ marginBottom: "0.5rem", color: "#94a3b8", fontSize: "0.9rem" }}>
          <strong>Window Size:</strong>{" "}
          {settings?.window_width ?? 1200} × {settings?.window_height ?? 800}
        </div>

        <button
          onClick={() => setShowResetConfirm(true)}
          aria-label="Reset all settings to defaults"
          style={{
            padding: "0.6rem 1rem",
            borderRadius: 8,
            border: "none",
            background: "linear-gradient(135deg, #f59e0b, #d97706)",
            color: "white",
            cursor: "pointer",
            fontWeight: "600",
          }}
        >
          🔄 Reset All Settings
        </button>
      </div>

      {/* App Info */}
      <div
        style={{
          padding: "1rem",
          borderRadius: 12,
          background: "rgba(2, 6, 23, 0.4)",
        }}
      >
        <h3
          style={{
            fontSize: "1.1rem",
            marginBottom: "0.75rem",
            color: "#e2e8f0",
          }}
        >
          ℹ️ About
        </h3>
        <p style={{ color: "#94a3b8", fontSize: "0.9rem" }}>
          IronDAG Desktop Wallet
        </p>
        <p style={{ color: "#64748b", fontSize: "0.85rem", marginTop: "0.5rem" }}>
          A secure desktop wallet for the IronDAG blockchain network.
        </p>
      </div>

      {/* Delete Keystore Confirmation Dialog */}
      <ConfirmDialog
        isOpen={showDeleteConfirm}
        title="Delete Keystore"
        message="Are you sure you want to delete your keystore? This action cannot be undone and you will lose access to your wallet unless you have a backup."
        onConfirm={handleDeleteKeystore}
        onCancel={() => setShowDeleteConfirm(false)}
      />

      {/* Reset Settings Confirmation Dialog */}
      <ConfirmDialog
        isOpen={showResetConfirm}
        title="Reset All Settings"
        message="Are you sure you want to reset all settings to their defaults? This will reset your RPC URL, startup preferences, and other settings. Your keystore will not be affected."
        onConfirm={handleResetSettings}
        onCancel={() => setShowResetConfirm(false)}
      />
    </section>
  );
};
