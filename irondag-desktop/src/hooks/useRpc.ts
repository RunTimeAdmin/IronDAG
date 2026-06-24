import { useState, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";

export function useRpc<T>(commandName: string, args?: Record<string, any>) {
  const [data, setData] = useState<T | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const execute = useCallback(async (overrideArgs?: Record<string, any>) => {
    setLoading(true);
    setError(null);
    try {
      const result = await invoke<T>(commandName, overrideArgs || args || {});
      setData(result);
      return result;
    } catch (e: any) {
      const errorMsg = e?.toString?.() ?? "Unknown error";
      setError(errorMsg);
      throw e;
    } finally {
      setLoading(false);
    }
  }, [commandName, args]);

  const clearError = useCallback(() => {
    setError(null);
  }, []);

  const clearData = useCallback(() => {
    setData(null);
  }, []);

  return { 
    data, 
    loading, 
    error, 
    execute, 
    setData,
    clearError,
    clearData
  };
}

// Hook for polling data at intervals
export function usePollingRpc<T>(
  commandName: string, 
  intervalMs: number = 10000,
  args?: Record<string, any>
) {
  const [data, setData] = useState<T | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const execute = useCallback(async (overrideArgs?: Record<string, any>) => {
    setLoading(true);
    setError(null);
    try {
      const result = await invoke<T>(commandName, overrideArgs || args || {});
      setData(result);
      setError(null);
      return result;
    } catch (e: any) {
      const errorMsg = e?.toString?.() ?? "Unknown error";
      setError(errorMsg);
      throw e;
    } finally {
      setLoading(false);
    }
  }, [commandName, args]);

  const startPolling = useCallback(() => {
    execute().catch(() => {}); // Initial call
    const interval = setInterval(() => {
      execute().catch(() => {}); // Silently fail on polling errors
    }, intervalMs);
    return () => clearInterval(interval);
  }, [execute, intervalMs]);

  return {
    data,
    loading,
    error,
    execute,
    startPolling,
    setData
  };
}
