import { useCallback, useEffect, useState } from 'react';
import { BrowserProvider, ethers } from 'ethers';
import { CHAIN_ID, CHAIN_ID_HEX, TICKER, NETWORK_NAME, DECIMALS, RPC_PUBLIC, EXPLORER_URL } from '../lib/config';

export interface WalletState {
  address: string | null;
  provider: BrowserProvider | null;
  connected: boolean;
  connect: () => Promise<void>;
  disconnect: () => void;
  addNetwork: () => Promise<void>;
}

declare global {
  interface Window {
    ethereum?: {
      request: (args: { method: string; params?: unknown[] }) => Promise<unknown>;
      on: (event: string, handler: (...args: unknown[]) => void) => void;
      removeListener: (event: string, handler: (...args: unknown[]) => void) => void;
      isMetaMask?: boolean;
    };
  }
}

export function useWallet(): WalletState {
  const [address, setAddress] = useState<string | null>(null);
  const [provider, setProvider] = useState<BrowserProvider | null>(null);

  const connect = useCallback(async () => {
    if (!window.ethereum) { alert('MetaMask not found. Please install it.'); return; }
    try {
      const accounts = await window.ethereum.request({ method: 'eth_requestAccounts' }) as string[];
      if (!accounts.length) return;

      // Switch/add network
      try {
        await window.ethereum.request({
          method: 'wallet_switchEthereumChain',
          params: [{ chainId: CHAIN_ID_HEX }],
        });
      } catch (err: unknown) {
        if ((err as { code?: number }).code === 4902) {
          await window.ethereum.request({
            method: 'wallet_addEthereumChain',
            params: [{
              chainId: CHAIN_ID_HEX,
              chainName: NETWORK_NAME,
              nativeCurrency: { name: TICKER, symbol: TICKER, decimals: DECIMALS },
              rpcUrls: [RPC_PUBLIC],
              blockExplorerUrls: [EXPLORER_URL],
            }],
          });
        }
      }

      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      const prov = new BrowserProvider(window.ethereum as any);
      setProvider(prov);
      setAddress(accounts[0]);
    } catch (e) {
      console.error('Wallet connect error:', e);
    }
  }, []);

  const disconnect = useCallback(() => {
    setAddress(null);
    setProvider(null);
  }, []);

  const addNetwork = useCallback(async () => {
    if (!window.ethereum) { alert('MetaMask not found.'); return; }
    try {
      await window.ethereum.request({
        method: 'wallet_addEthereumChain',
        params: [{
          chainId: CHAIN_ID_HEX,
          chainName: NETWORK_NAME,
          nativeCurrency: { name: TICKER, symbol: TICKER, decimals: DECIMALS },
          rpcUrls: [RPC_PUBLIC],
          blockExplorerUrls: [EXPLORER_URL],
        }],
      });
    } catch (e) {
      console.error('Add network error:', e);
    }
  }, []);

  useEffect(() => {
    if (!window.ethereum) return;
    const handler = (accounts: unknown) => {
      const accs = accounts as string[];
      if (accs.length === 0) disconnect();
      else setAddress(accs[0]);
    };
    window.ethereum.on('accountsChanged', handler);
    return () => window.ethereum?.removeListener('accountsChanged', handler);
  }, [disconnect]);

  // Restore from existing connection
  useEffect(() => {
    if (!window.ethereum) return;
    window.ethereum.request({ method: 'eth_accounts' }).then((accounts) => {
      const accs = accounts as string[];
      if (accs.length) {
        setAddress(accs[0]);
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        setProvider(new BrowserProvider(window.ethereum as any));
      }
    }).catch(() => {});
  }, []);

  return { address, provider, connected: !!address, connect, disconnect, addNetwork };
}

export async function sendIDAG(provider: BrowserProvider, to: string, amountEther: string): Promise<string> {
  const signer = await provider.getSigner();
  const tx = await signer.sendTransaction({
    to,
    value: ethers.parseEther(amountEther),
  });
  return tx.hash;
}

export async function deployContract(
  provider: BrowserProvider,
  bytecode: string,
  abi: object[],
  ctorArgs: unknown[],
): Promise<string> {
  const signer = await provider.getSigner();
  const factory = new ethers.ContractFactory(abi, bytecode, signer);
  const contract = await factory.deploy(...ctorArgs);
  await contract.waitForDeployment();
  return await contract.getAddress();
}
