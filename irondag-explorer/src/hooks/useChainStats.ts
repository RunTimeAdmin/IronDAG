import { useEffect, useRef, useState } from 'react';
import { rpcCall, rpcFast } from '../lib/rpc';
import { hexToNum, DagStats, RpcBlock } from '../lib/format';
import { DASHBOARD_REFRESH_MS } from '../lib/config';

export interface ChainStats {
  blockNumber: number;
  dagStats: DagStats | null;
  peerCount: number;
  avgBlockTimeSec: number | null;
  blockRate: number | null;
  tps: number | null;
  finalizedBlock: number | null;
  streamA: number;
  streamB: number | string;
}

const CACHE_BLOCKS = 20;

export function useChainStats() {
  const [stats, setStats] = useState<ChainStats>({
    blockNumber: 0, dagStats: null, peerCount: 0,
    avgBlockTimeSec: null, blockRate: null, tps: null,
    finalizedBlock: null, streamA: 0, streamB: 0,
  });

  // Persistent caches across polls
  const cache = useRef({
    blockNumber: 0,
    dagStats: null as DagStats | null,
    peerCount: 0,
    finalizedBlock: null as number | null,
    blockWindow: [] as { number: number; timestamp: number; txCount: number }[],
  });

  useEffect(() => {
    let cancelled = false;

    async function poll() {
      if (cancelled) return;
      try {
        const [bnHex, dagRaw, peerHex] = await Promise.allSettled([
          rpcCall<string>('eth_blockNumber'),
          rpcFast<DagStats>('irondag_getDagStats'),
          rpcFast<string>('net_peerCount'),
        ]);

        const bn = bnHex.status === 'fulfilled' ? hexToNum(bnHex.value) : cache.current.blockNumber;
        if (bn > 0) cache.current.blockNumber = bn;

        const dagStats = dagRaw.status === 'fulfilled' && dagRaw.value ? dagRaw.value : cache.current.dagStats;
        if (dagStats) cache.current.dagStats = dagStats;

        const peers = peerHex.status === 'fulfilled' ? hexToNum(peerHex.value) : cache.current.peerCount;
        if (peers >= 0) cache.current.peerCount = peers;

        // Fetch finalized
        let finalized = cache.current.finalizedBlock;
        try {
          const fin = await rpcCall<RpcBlock>('eth_getBlockByNumber', ['finalized', false]);
          if (fin?.number) { finalized = hexToNum(fin.number); cache.current.finalizedBlock = finalized; }
        } catch { /* keep cached */ }

        // Update rolling block window for timing stats
        if (bn > 0 && bn > (cache.current.blockWindow[0]?.number ?? -1)) {
          const want = Math.min(CACHE_BLOCKS, bn + 1);
          const toFetch: Promise<RpcBlock | null>[] = [];
          for (let i = 0; i < want; i++) {
            const num = bn - i;
            if (!cache.current.blockWindow.find(b => b.number === num)) {
              toFetch.push(rpcCall<RpcBlock>('eth_getBlockByNumber', [`0x${num.toString(16)}`, false]).catch(() => null));
            }
          }
          const fetched = await Promise.all(toFetch);
          for (const b of fetched) {
            if (!b) continue;
            cache.current.blockWindow.push({
              number: hexToNum(b.number),
              timestamp: hexToNum(b.timestamp),
              txCount: Array.isArray(b.transactions) ? b.transactions.length : 0,
            });
          }
          // Keep only the latest CACHE_BLOCKS
          cache.current.blockWindow.sort((a, b) => b.number - a.number);
          cache.current.blockWindow = cache.current.blockWindow.slice(0, CACHE_BLOCKS);
        }

        const window = cache.current.blockWindow;
        let avgBlockTimeSec: number | null = null;
        let blockRate: number | null = null;
        let tps: number | null = null;

        if (window.length >= 2) {
          const sorted = [...window].sort((a, b) => a.number - b.number);
          const diffs: number[] = [];
          for (let i = 1; i < sorted.length; i++) {
            const d = Math.abs(sorted[i].timestamp - sorted[i - 1].timestamp);
            if (d > 0 && d < 3600) diffs.push(d);
          }
          if (diffs.length > 0) {
            diffs.sort((a, b) => a - b);
            const median = diffs[Math.floor(diffs.length / 2)];
            avgBlockTimeSec = median;
            blockRate = median > 0 ? 1 / median : 0;
          }
          const totalTx = sorted.reduce((s, b) => s + b.txCount, 0);
          const span = sorted[sorted.length - 1].timestamp - sorted[0].timestamp;
          if (span > 0) tps = totalTx / span;
        }

        // Stream counts
        let streamA = 0;
        let streamB: number | string = 0;
        try {
          const counts = await rpcCall<{ A: number; B: number }>('irondag_getStreamCounts', [], { retries: 1, quiet: true });
          if (counts) { streamA = counts.A ?? 0; streamB = counts.B ?? 0; }
        } catch {
          // fallback: use dagStats total as approximation
          if (dagStats) {
            const total = dagStats.total_blocks;
            streamA = Math.round(total * 0.6);
            streamB = total - streamA;
          }
        }

        if (!cancelled) {
          setStats({
            blockNumber: cache.current.blockNumber,
            dagStats: cache.current.dagStats,
            peerCount: cache.current.peerCount,
            avgBlockTimeSec,
            blockRate,
            tps,
            finalizedBlock: finalized,
            streamA,
            streamB,
          });
        }
      } catch (e) {
        console.error('Stats poll error:', e);
      }
    }

    poll();
    const id = setInterval(poll, DASHBOARD_REFRESH_MS);
    return () => { cancelled = true; clearInterval(id); };
  }, []);

  return stats;
}
