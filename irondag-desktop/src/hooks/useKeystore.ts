import { useState, useEffect, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";

export function useKeystore() {
  const [hasKeystore, setHasKeystore] = useState(false);
  const [isUnlocked, setIsUnlocked] = useState(false);
  const [walletAddress, setWalletAddress] = useState<string>("");
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Check keystore status on mount
  useEffect(() => {
    checkKeystoreStatus();
  }, []);

  const checkKeystoreStatus = useCallback(async () => {
    try {
      const has = await invoke<boolean>("has_keystore");
      setHasKeystore(has);
      if (has) {
        // Try to get wallet address if unlocked
        try {
          const addr = await invoke<string>("get_wallet_address");
          setWalletAddress(addr);
          setIsUnlocked(true);
        } catch {
          setIsUnlocked(false);
        }
      }
    } catch (e: any) {
      console.error("Failed to check keystore status:", e);
    }
  }, []);

  const createKeystore = useCallback(async (password: string) => {
    setLoading(true);
    setError(null);
    try {
      await invoke("create_keystore", { password });
      setHasKeystore(true);
      await checkKeystoreStatus();
    } catch (e: any) {
      const errorMsg = e?.toString?.() ?? "Failed to create keystore";
      setError(errorMsg);
      throw e;
    } finally {
      setLoading(false);
    }
  }, [checkKeystoreStatus]);

  const unlockKeystore = useCallback(async (password: string) => {
    setLoading(true);
    setError(null);
    try {
      await invoke("unlock_keystore", { password });
      setIsUnlocked(true);
      const addr = await invoke<string>("get_wallet_address");
      setWalletAddress(addr);
    } catch (e: any) {
      const errorMsg = e?.toString?.() ?? "Failed to unlock keystore";
      setError(errorMsg);
      setIsUnlocked(false);
      throw e;
    } finally {
      setLoading(false);
    }
  }, []);

  const lockKeystore = useCallback(async () => {
    setLoading(true);
    try {
      await invoke("lock_keystore");
      setIsUnlocked(false);
      setWalletAddress("");
    } catch (e: any) {
      const errorMsg = e?.toString?.() ?? "Failed to lock keystore";
      setError(errorMsg);
    } finally {
      setLoading(false);
    }
  }, []);

  const deleteKeystore = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      await invoke("delete_keystore");
      setHasKeystore(false);
      setIsUnlocked(false);
      setWalletAddress("");
    } catch (e: any) {
      const errorMsg = e?.toString?.() ?? "Failed to delete keystore";
      setError(errorMsg);
      throw e;
    } finally {
      setLoading(false);
    }
  }, []);

  const createNewKey = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const addr = await invoke<string>("create_new_key");
      setWalletAddress(addr);
      setIsUnlocked(true);
      return addr;
    } catch (e: any) {
      const errorMsg = e?.toString?.() ?? "Failed to create key";
      setError(errorMsg);
      throw e;
    } finally {
      setLoading(false);
    }
  }, []);

  const loadWalletAddress = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const addr = await invoke<string>("get_wallet_address");
      setWalletAddress(addr);
      setIsUnlocked(true);
      return addr;
    } catch (e: any) {
      const errorMsg = e?.toString?.() ?? "No key loaded";
      setError(errorMsg);
      setIsUnlocked(false);
      throw e;
    } finally {
      setLoading(false);
    }
  }, []);

  const clearError = useCallback(() => {
    setError(null);
  }, []);

  return {
    hasKeystore,
    isUnlocked,
    walletAddress,
    loading,
    error,
    createKeystore,
    unlockKeystore,
    lockKeystore,
    deleteKeystore,
    createNewKey,
    loadWalletAddress,
    checkKeystoreStatus,
    clearError
  };
}
