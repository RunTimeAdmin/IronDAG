import React, { useState, useEffect, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { TestDefinition, TestResult } from "../types";

interface TestRunnerProps {
  setError: (error: string | null) => void;
}

export const TestRunner: React.FC<TestRunnerProps> = ({ setError }) => {
  const [tests, setTests] = useState<TestDefinition[]>([]);
  const [selectedTest, setSelectedTest] = useState<string>("");
  const [testRunning, setTestRunning] = useState<boolean>(false);
  const [testResult, setTestResult] = useState<TestResult | null>(null);

  const loadTests = useCallback(async () => {
    try {
      const available = await invoke<TestDefinition[]>("list_tests");
      setTests(available);
      if (!selectedTest && available.length > 0) {
        setSelectedTest(available[0].name);
      }
    } catch (e: any) {
      setError(e?.toString?.() ?? "Failed to load tests");
    }
  }, [selectedTest, setError]);

  useEffect(() => {
    loadTests();
  }, [loadTests]);

  const runSelectedTest = async () => {
    if (!selectedTest) {
      setError("Select a test to run");
      return;
    }
    setTestRunning(true);
    setError(null);
    setTestResult(null);
    try {
      const result = await invoke<TestResult>("run_test", { name: selectedTest });
      setTestResult(result);
    } catch (e: any) {
      setError(e?.toString?.() ?? "Failed to run test");
    } finally {
      setTestRunning(false);
    }
  };

  return (
    <section
      style={{
        padding: "1.5rem",
        borderRadius: 16,
        background: "rgba(30, 41, 59, 0.7)",
        backdropFilter: "blur(12px)",
        border: "1px solid rgba(236, 72, 153, 0.2)",
        boxShadow: "0 8px 32px rgba(0, 0, 0, 0.3)",
      }}
    >
      <h2 style={{ fontSize: "1.4rem", marginBottom: "1rem", fontWeight: "600", color: "#f8fafc" }}>
        🧪 Test Runner
      </h2>
      <div style={{ display: "flex", gap: "0.75rem", flexWrap: "wrap", alignItems: "center" }}>
        <select
          value={selectedTest}
          onChange={(e) => setSelectedTest(e.target.value)}
          style={{
            padding: "0.65rem",
            borderRadius: 8,
            border: "1px solid rgba(236, 72, 153, 0.3)",
            background: "rgba(2, 6, 23, 0.6)",
            color: "#e5e7eb",
            minWidth: 240,
          }}
        >
          {tests.map((test) => (
            <option key={test.name} value={test.name}>
              {test.label}
            </option>
          ))}
        </select>
        <button
          onClick={runSelectedTest}
          disabled={testRunning || !selectedTest}
          style={{
            padding: "0.65rem 1.5rem",
            borderRadius: 8,
            border: "none",
            background: testRunning ? "rgba(236, 72, 153, 0.5)" : "linear-gradient(135deg, #ec4899, #db2777)",
            color: "white",
            cursor: testRunning ? "not-allowed" : "pointer",
            fontWeight: "600",
          }}
        >
          {testRunning ? "⏳ Running..." : "▶️ Run Test"}
        </button>
      </div>
      {selectedTest && (
        <p style={{ marginTop: "0.75rem", color: "#94a3b8" }}>
          {tests.find((test) => test.name === selectedTest)?.description ?? ""}
        </p>
      )}
      {testResult && (
        <div style={{ marginTop: "1rem" }}>
          <div style={{ color: testResult.exit_code === 0 ? "#10b981" : "#f87171" }}>
            Result: {testResult.exit_code === 0 ? "PASS" : "FAIL"} · Exit {testResult.exit_code} · {testResult.duration_ms} ms
          </div>
          <pre
            style={{
              marginTop: "0.75rem",
              padding: "0.75rem",
              borderRadius: 8,
              background: "rgba(2, 6, 23, 0.7)",
              color: "#e2e8f0",
              maxHeight: 240,
              overflow: "auto",
              whiteSpace: "pre-wrap",
              fontSize: "0.85rem",
            }}
          >
            {testResult.stdout || "(no stdout)"}
            {testResult.stderr ? `\n\n[stderr]\n${testResult.stderr}` : ""}
          </pre>
        </div>
      )}
    </section>
  );
};
