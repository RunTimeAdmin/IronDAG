import React, { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { TabType, PrivacyStats, PriceFeed, RecurringTransaction, StopLossOrder } from "../types";

interface AdvancedFeaturesProps {
  activeTab: TabType;
  setError: (error: string | null) => void;
  setConfirmDialog: (dialog: { title: string; message: string; onConfirm: () => void } | null) => void;
}

export const AdvancedFeatures: React.FC<AdvancedFeaturesProps> = ({ activeTab, setError }) => {
  // Privacy state
  const [privacyStats, setPrivacyStats] = useState<PrivacyStats | null>(null);
  const [privacyFrom, setPrivacyFrom] = useState<string>("");
  const [privacyTo, setPrivacyTo] = useState<string>("");
  const [privacyAmount, setPrivacyAmount] = useState<string>("");
  const [, setPrivacyProof] = useState<string | null>(null);
  const [privacyLoading, setPrivacyLoading] = useState(false);

  // Oracle state
  const [priceFeeds, setPriceFeeds] = useState<PriceFeed[]>([]);
  const [randomnessRequest, setRandomnessRequest] = useState<string>("");
  const [randomnessResult, setRandomnessResult] = useState<string | null>(null);
  const [oraclesLoading, setOraclesLoading] = useState(false);
  const [randomnessLoading, setRandomnessLoading] = useState(false);

  // Recurring state
  const [recurringTxs, setRecurringTxs] = useState<RecurringTransaction[]>([]);
  const [recurringFrom, setRecurringFrom] = useState<string>("");
  const [recurringTo, setRecurringTo] = useState<string>("");
  const [recurringAmount, setRecurringAmount] = useState<string>("");
  const [recurringInterval, setRecurringInterval] = useState<string>("");
  const [recurringLoading, setRecurringLoading] = useState(false);

  // Stop-loss state
  const [stopLossOrders, setStopLossOrders] = useState<StopLossOrder[]>([]);
  const [stopLossToken, setStopLossToken] = useState<string>("");
  const [stopLossAmount, setStopLossAmount] = useState<string>("");
  const [stopLossTriggerPrice, setStopLossTriggerPrice] = useState<string>("");
  const [stopLossOrderType, setStopLossOrderType] = useState<"sell" | "buy">("sell");
  const [stopLossLoading, setStopLossLoading] = useState(false);

  const loadPrivacyStats = async () => {
    setPrivacyLoading(true);
    try {
      const stats = await invoke<PrivacyStats>("irondag_get_privacy_stats");
      setPrivacyStats(stats);
    } catch (e: any) {
      setError(e?.toString?.() ?? "Failed to load privacy stats");
    } finally {
      setPrivacyLoading(false);
    }
  };

  const createPrivateTransaction = async () => {
    if (!privacyFrom || !privacyTo || !privacyAmount) {
      setError("Please fill in all fields");
      return;
    }
    setPrivacyLoading(true);
    try {
      const result = await invoke<any>("irondag_create_private_transaction", {
        from: privacyFrom,
        to: privacyTo,
        amount: privacyAmount,
      });
      setPrivacyProof(result.proof || null);
    } catch (e: any) {
      setError(e?.toString?.() ?? "Failed to create private transaction");
    } finally {
      setPrivacyLoading(false);
    }
  };

  const loadPriceFeeds = async () => {
    setOraclesLoading(true);
    try {
      const feeds = await invoke<{ price_feeds: PriceFeed[] }>("irondag_get_price_feeds");
      setPriceFeeds(feeds.price_feeds || []);
    } catch (e: any) {
      setError(e?.toString?.() ?? "Failed to load price feeds");
    } finally {
      setOraclesLoading(false);
    }
  };

  const handleRandomness = async () => {
    setRandomnessLoading(true);
    try {
      if (randomnessRequest) {
        const result = await invoke<any>("irondag_get_randomness", { request_id: randomnessRequest });
        setRandomnessResult(result.randomness || null);
      } else {
        const result = await invoke<any>("irondag_request_randomness", {});
        setRandomnessResult(result.request_id || null);
      }
    } catch (e: any) {
      setError(e?.toString?.() ?? "Failed to request randomness");
    } finally {
      setRandomnessLoading(false);
    }
  };

  const loadRecurringTxs = async () => {
    if (!recurringFrom) {
      setError("Please enter your address");
      return;
    }
    setRecurringLoading(true);
    try {
      const txs = await invoke<{ recurring_transactions: RecurringTransaction[] }>("irondag_get_recurring_transactions", { address: recurringFrom });
      setRecurringTxs(txs.recurring_transactions || []);
    } catch (e: any) {
      setError(e?.toString?.() ?? "Failed to load recurring transactions");
    } finally {
      setRecurringLoading(false);
    }
  };

  const createRecurringTx = async () => {
    if (!recurringFrom || !recurringTo || !recurringAmount || !recurringInterval) {
      setError("Please fill in all fields");
      return;
    }
    setRecurringLoading(true);
    try {
      await invoke<any>("irondag_create_recurring_transaction", {
        from: recurringFrom,
        to: recurringTo,
        value: recurringAmount,
        interval_seconds: parseInt(recurringInterval),
      });
      setRecurringFrom("");
      setRecurringTo("");
      setRecurringAmount("");
      setRecurringInterval("");
    } catch (e: any) {
      setError(e?.toString?.() ?? "Failed to create recurring transaction");
    } finally {
      setRecurringLoading(false);
    }
  };

  const loadStopLossOrders = async () => {
    setStopLossLoading(true);
    try {
      const orders = await invoke<{ stop_loss_orders: StopLossOrder[] }>("irondag_get_stop_loss_orders", { address: "" });
      setStopLossOrders(orders.stop_loss_orders || []);
    } catch (e: any) {
      setError(e?.toString?.() ?? "Failed to load stop-loss orders");
    } finally {
      setStopLossLoading(false);
    }
  };

  const createStopLoss = async () => {
    if (!stopLossToken || !stopLossAmount || !stopLossTriggerPrice) {
      setError("Please fill in all fields");
      return;
    }
    setStopLossLoading(true);
    try {
      await invoke<any>("irondag_create_stop_loss", {
        token_symbol: stopLossToken,
        amount: stopLossAmount,
        trigger_price: stopLossTriggerPrice,
        order_type: stopLossOrderType,
      });
      setStopLossToken("");
      setStopLossAmount("");
      setStopLossTriggerPrice("");
    } catch (e: any) {
      setError(e?.toString?.() ?? "Failed to create stop-loss order");
    } finally {
      setStopLossLoading(false);
    }
  };

  const renderPrivacyTab = () => (
    <>
      <h3 style={{ fontSize: "1.2rem", marginBottom: "1rem", fontWeight: "600", color: "#f8fafc" }}>
        🔒 Privacy Transactions
      </h3>
      <button
        onClick={loadPrivacyStats}
        disabled={privacyLoading}
        style={{
          padding: "0.65rem 1.5rem",
          borderRadius: 8,
          border: "none",
          background: privacyLoading ? "rgba(236, 72, 153, 0.5)" : "linear-gradient(135deg, #ec4899, #db2777)",
          color: "white",
          cursor: privacyLoading ? "not-allowed" : "pointer",
          fontWeight: "600",
          marginBottom: "1rem",
        }}
      >
        {privacyLoading ? "⏳ Loading..." : "📊 Load Privacy Stats"}
      </button>
      {privacyStats && (
        <div style={{
          padding: "1rem",
          background: "rgba(236, 72, 153, 0.1)",
          border: "1px solid rgba(236, 72, 153, 0.2)",
          borderRadius: 10,
          marginBottom: "1.5rem"
        }}>
          <div style={{ color: "#94a3b8", marginBottom: "0.5rem" }}>
            <strong>Total Private Transactions:</strong> {privacyStats.total_private_txs || 0}
          </div>
          <div style={{ color: "#94a3b8" }}>
            <strong>Privacy Enabled:</strong> {privacyStats.enabled ? "✅ Yes" : "❌ No"}
          </div>
        </div>
      )}
      <div style={{ display: "grid", gap: "1rem" }}>
        <input
          type="text"
          value={privacyFrom}
          onChange={(e) => setPrivacyFrom(e.target.value)}
          placeholder="From Address (0x...)"
          style={{
            width: "100%",
            padding: "0.75rem",
            borderRadius: 8,
            border: "1px solid rgba(236, 72, 153, 0.3)",
            background: "rgba(2, 6, 23, 0.6)",
            color: "#e5e7eb",
            fontFamily: "'JetBrains Mono', monospace",
          }}
        />
        <input
          type="text"
          value={privacyTo}
          onChange={(e) => setPrivacyTo(e.target.value)}
          placeholder="To Address (0x...)"
          style={{
            width: "100%",
            padding: "0.75rem",
            borderRadius: 8,
            border: "1px solid rgba(236, 72, 153, 0.3)",
            background: "rgba(2, 6, 23, 0.6)",
            color: "#e5e7eb",
            fontFamily: "'JetBrains Mono', monospace",
          }}
        />
        <input
          type="text"
          value={privacyAmount}
          onChange={(e) => setPrivacyAmount(e.target.value)}
          placeholder="Amount (IDAG)"
          style={{
            width: "100%",
            padding: "0.75rem",
            borderRadius: 8,
            border: "1px solid rgba(236, 72, 153, 0.3)",
            background: "rgba(2, 6, 23, 0.6)",
            color: "#e5e7eb",
          }}
        />
        <button
          onClick={createPrivateTransaction}
          disabled={privacyLoading || !privacyFrom || !privacyTo || !privacyAmount}
          style={{
            padding: "0.75rem 2rem",
            borderRadius: 8,
            border: "none",
            background: (!privacyFrom || !privacyTo || !privacyAmount || privacyLoading)
              ? "rgba(236, 72, 153, 0.5)"
              : "linear-gradient(135deg, #ec4899, #db2777)",
            color: "white",
            cursor: (!privacyFrom || !privacyTo || !privacyAmount || privacyLoading) ? "not-allowed" : "pointer",
            fontWeight: "600",
            width: "100%",
          }}
        >
          {privacyLoading ? "⏳ Creating..." : "🔒 Create Private Transaction"}
        </button>
      </div>
    </>
  );

  const renderOraclesTab = () => (
    <>
      <h3 style={{ fontSize: "1.2rem", marginBottom: "1rem", fontWeight: "600", color: "#f8fafc" }}>
        🔮 Oracle Network
      </h3>
      <button
        onClick={loadPriceFeeds}
        disabled={oraclesLoading}
        style={{
          padding: "0.65rem 1.5rem",
          borderRadius: 8,
          border: "none",
          background: oraclesLoading ? "rgba(6, 182, 212, 0.5)" : "linear-gradient(135deg, #06b6d4, #0891b2)",
          color: "white",
          cursor: oraclesLoading ? "not-allowed" : "pointer",
          fontWeight: "600",
          marginBottom: "1rem",
        }}
      >
        {oraclesLoading ? "⏳ Loading..." : "🔄 Refresh Price Feeds"}
      </button>
      {priceFeeds.length > 0 && (
        <div style={{ display: "grid", gap: "0.75rem", marginBottom: "1.5rem" }}>
          {priceFeeds.map((feed, idx) => (
            <div
              key={idx}
              style={{
                padding: "1rem",
                background: "rgba(6, 182, 212, 0.1)",
                border: "1px solid rgba(6, 182, 212, 0.2)",
                borderRadius: 10,
              }}
            >
              <div style={{ color: "#06b6d4", fontWeight: "600" }}>
                {feed.symbol || feed.feed_id}
              </div>
              <div style={{ color: "#94a3b8", fontSize: "0.9rem" }}>
                Price: {feed.price || "N/A"}
              </div>
            </div>
          ))}
        </div>
      )}
      <h4 style={{ fontSize: "1.1rem", marginBottom: "1rem", color: "#f8fafc" }}>
        Verifiable Randomness (VRF)
      </h4>
      <input
        type="text"
        value={randomnessRequest}
        onChange={(e) => setRandomnessRequest(e.target.value)}
        placeholder="Request ID (optional)"
        style={{
          width: "100%",
          padding: "0.75rem",
          borderRadius: 8,
          border: "1px solid rgba(6, 182, 212, 0.3)",
          background: "rgba(2, 6, 23, 0.6)",
          color: "#e5e7eb",
          marginBottom: "1rem",
        }}
      />
      <button
        onClick={handleRandomness}
        disabled={randomnessLoading}
        style={{
          padding: "0.75rem 2rem",
          borderRadius: 8,
          border: "none",
          background: randomnessLoading ? "rgba(6, 182, 212, 0.5)" : "linear-gradient(135deg, #06b6d4, #0891b2)",
          color: "white",
          cursor: randomnessLoading ? "not-allowed" : "pointer",
          fontWeight: "600",
          width: "100%",
        }}
      >
        {randomnessLoading ? "⏳ Processing..." : randomnessRequest ? "🔍 Get Randomness" : "🎲 Request Randomness"}
      </button>
      {randomnessResult && (
        <div style={{
          marginTop: "1rem",
          padding: "1rem",
          background: "rgba(6, 182, 212, 0.1)",
          border: "1px solid rgba(6, 182, 212, 0.2)",
          borderRadius: 10,
          wordBreak: "break-all",
          fontFamily: "'JetBrains Mono', monospace",
          color: "#94a3b8"
        }}>
          <strong style={{ color: "#06b6d4" }}>Result:</strong> {randomnessResult}
        </div>
      )}
    </>
  );

  const renderRecurringTab = () => (
    <>
      <h3 style={{ fontSize: "1.2rem", marginBottom: "1rem", fontWeight: "600", color: "#f8fafc" }}>
        🔄 Recurring Transactions
      </h3>
      <button
        onClick={loadRecurringTxs}
        disabled={recurringLoading || !recurringFrom}
        style={{
          padding: "0.65rem 1.5rem",
          borderRadius: 8,
          border: "none",
          background: (recurringLoading || !recurringFrom)
            ? "rgba(16, 185, 129, 0.5)"
            : "linear-gradient(135deg, #10b981, #059669)",
          color: "white",
          cursor: (recurringLoading || !recurringFrom) ? "not-allowed" : "pointer",
          fontWeight: "600",
          marginBottom: "1rem",
        }}
      >
        {recurringLoading ? "⏳ Loading..." : "🔄 Load Recurring Transactions"}
      </button>
      {recurringTxs.length > 0 && (
        <div style={{ display: "grid", gap: "0.75rem", marginBottom: "1.5rem" }}>
          {recurringTxs.map((tx, idx) => (
            <div
              key={idx}
              style={{
                padding: "1rem",
                background: "rgba(16, 185, 129, 0.1)",
                border: "1px solid rgba(16, 185, 129, 0.2)",
                borderRadius: 10,
              }}
            >
              <div style={{ color: "#10b981", fontWeight: "600", marginBottom: "0.5rem" }}>
                ID: {tx.recurring_tx_id?.substring(0, 20)}...
              </div>
              <div style={{ color: "#94a3b8", fontSize: "0.9rem" }}>
                Amount: {tx.value} IDAG | Status: {tx.status}
              </div>
            </div>
          ))}
        </div>
      )}
      <h4 style={{ fontSize: "1.1rem", marginBottom: "1rem", color: "#f8fafc" }}>
        Create Recurring Transaction
      </h4>
      <div style={{ display: "grid", gap: "1rem" }}>
        <input
          type="text"
          value={recurringFrom}
          onChange={(e) => setRecurringFrom(e.target.value)}
          placeholder="From Address (0x...)"
          style={{
            width: "100%",
            padding: "0.75rem",
            borderRadius: 8,
            border: "1px solid rgba(16, 185, 129, 0.3)",
            background: "rgba(2, 6, 23, 0.6)",
            color: "#e5e7eb",
            fontFamily: "'JetBrains Mono', monospace",
          }}
        />
        <input
          type="text"
          value={recurringTo}
          onChange={(e) => setRecurringTo(e.target.value)}
          placeholder="To Address (0x...)"
          style={{
            width: "100%",
            padding: "0.75rem",
            borderRadius: 8,
            border: "1px solid rgba(16, 185, 129, 0.3)",
            background: "rgba(2, 6, 23, 0.6)",
            color: "#e5e7eb",
            fontFamily: "'JetBrains Mono', monospace",
          }}
        />
        <input
          type="text"
          value={recurringAmount}
          onChange={(e) => setRecurringAmount(e.target.value)}
          placeholder="Amount (IDAG)"
          style={{
            width: "100%",
            padding: "0.75rem",
            borderRadius: 8,
            border: "1px solid rgba(16, 185, 129, 0.3)",
            background: "rgba(2, 6, 23, 0.6)",
            color: "#e5e7eb",
          }}
        />
        <input
          type="text"
          value={recurringInterval}
          onChange={(e) => setRecurringInterval(e.target.value)}
          placeholder="Interval (seconds, e.g., 3600 for 1 hour)"
          style={{
            width: "100%",
            padding: "0.75rem",
            borderRadius: 8,
            border: "1px solid rgba(16, 185, 129, 0.3)",
            background: "rgba(2, 6, 23, 0.6)",
            color: "#e5e7eb",
          }}
        />
        <button
          onClick={createRecurringTx}
          disabled={recurringLoading || !recurringFrom || !recurringTo || !recurringAmount || !recurringInterval}
          style={{
            padding: "0.75rem 2rem",
            borderRadius: 8,
            border: "none",
            background: (!recurringFrom || !recurringTo || !recurringAmount || !recurringInterval || recurringLoading)
              ? "rgba(16, 185, 129, 0.5)"
              : "linear-gradient(135deg, #10b981, #059669)",
            color: "white",
            cursor: (!recurringFrom || !recurringTo || !recurringAmount || !recurringInterval || recurringLoading) ? "not-allowed" : "pointer",
            fontWeight: "600",
            width: "100%",
          }}
        >
          {recurringLoading ? "⏳ Creating..." : "🔄 Create Recurring Transaction"}
        </button>
      </div>
    </>
  );

  const renderStopLossTab = () => (
    <>
      <h3 style={{ fontSize: "1.2rem", marginBottom: "1rem", fontWeight: "600", color: "#f8fafc" }}>
        ⚠️ Stop-Loss Orders
      </h3>
      <button
        onClick={loadStopLossOrders}
        disabled={stopLossLoading}
        style={{
          padding: "0.65rem 1.5rem",
          borderRadius: 8,
          border: "none",
          background: stopLossLoading ? "rgba(245, 158, 11, 0.5)" : "linear-gradient(135deg, #f59e0b, #d97706)",
          color: "white",
          cursor: stopLossLoading ? "not-allowed" : "pointer",
          fontWeight: "600",
          marginBottom: "1rem",
        }}
      >
        {stopLossLoading ? "⏳ Loading..." : "🔄 Load Stop-Loss Orders"}
      </button>
      {stopLossOrders.length > 0 && (
        <div style={{ display: "grid", gap: "0.75rem", marginBottom: "1.5rem" }}>
          {stopLossOrders.map((order, idx) => (
            <div
              key={idx}
              style={{
                padding: "1rem",
                background: "rgba(245, 158, 11, 0.1)",
                border: "1px solid rgba(245, 158, 11, 0.2)",
                borderRadius: 10,
              }}
            >
              <div style={{ color: "#f59e0b", fontWeight: "600", marginBottom: "0.5rem" }}>
                Order: {order.token_symbol || "N/A"}
              </div>
              <div style={{ color: "#94a3b8", fontSize: "0.9rem" }}>
                Amount: {order.amount} | Trigger: {order.trigger_price}
              </div>
            </div>
          ))}
        </div>
      )}
      <h4 style={{ fontSize: "1.1rem", marginBottom: "1rem", color: "#f8fafc" }}>
        Create Stop-Loss Order
      </h4>
      <div style={{ display: "grid", gap: "1rem" }}>
        <input
          type="text"
          value={stopLossToken}
          onChange={(e) => setStopLossToken(e.target.value)}
          placeholder="Token Symbol (e.g., IDAG)"
          style={{
            width: "100%",
            padding: "0.75rem",
            borderRadius: 8,
            border: "1px solid rgba(245, 158, 11, 0.3)",
            background: "rgba(2, 6, 23, 0.6)",
            color: "#e5e7eb",
          }}
        />
        <input
          type="text"
          value={stopLossAmount}
          onChange={(e) => setStopLossAmount(e.target.value)}
          placeholder="Amount"
          style={{
            width: "100%",
            padding: "0.75rem",
            borderRadius: 8,
            border: "1px solid rgba(245, 158, 11, 0.3)",
            background: "rgba(2, 6, 23, 0.6)",
            color: "#e5e7eb",
          }}
        />
        <input
          type="text"
          value={stopLossTriggerPrice}
          onChange={(e) => setStopLossTriggerPrice(e.target.value)}
          placeholder="Trigger Price"
          style={{
            width: "100%",
            padding: "0.75rem",
            borderRadius: 8,
            border: "1px solid rgba(245, 158, 11, 0.3)",
            background: "rgba(2, 6, 23, 0.6)",
            color: "#e5e7eb",
          }}
        />
        <select
          value={stopLossOrderType}
          onChange={(e) => setStopLossOrderType(e.target.value as "sell" | "buy")}
          style={{
            width: "100%",
            padding: "0.75rem",
            borderRadius: 8,
            border: "1px solid rgba(245, 158, 11, 0.3)",
            background: "rgba(2, 6, 23, 0.6)",
            color: "#e5e7eb",
          }}
        >
          <option value="sell">Sell (when price drops)</option>
          <option value="buy">Buy (when price rises)</option>
        </select>
        <button
          onClick={createStopLoss}
          disabled={stopLossLoading || !stopLossToken || !stopLossAmount || !stopLossTriggerPrice}
          style={{
            padding: "0.75rem 2rem",
            borderRadius: 8,
            border: "none",
            background: (!stopLossToken || !stopLossAmount || !stopLossTriggerPrice || stopLossLoading)
              ? "rgba(245, 158, 11, 0.5)"
              : "linear-gradient(135deg, #f59e0b, #d97706)",
            color: "white",
            cursor: (!stopLossToken || !stopLossAmount || !stopLossTriggerPrice || stopLossLoading) ? "not-allowed" : "pointer",
            fontWeight: "600",
            width: "100%",
          }}
        >
          {stopLossLoading ? "⏳ Creating..." : "⚠️ Create Stop-Loss Order"}
        </button>
      </div>
    </>
  );

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
      {activeTab === "privacy" && renderPrivacyTab()}
      {activeTab === "oracles" && renderOraclesTab()}
      {activeTab === "recurring" && renderRecurringTab()}
      {activeTab === "stop-loss" && renderStopLossTab()}
    </section>
  );
};
