import { useCallback, useEffect, useRef, useState } from 'react';
import { rpcCall } from '../lib/rpc';
import { hexToNum, RpcBlock, RpcTx, getStreamType } from '../lib/format';
import { BLOCKS_REFRESH_MS } from '../lib/config';

export type StreamFilter = 'all' | 'A' | 'B';

export interface BlockRow {
  number: number;
  hash: string;
  timestamp: number;
  txCount: number;
  miner: string;
  stream: 'A' | 'B';
  gasUsed: number;
  gasLimit: number;
}

export interface TxRow {
  hash: string;
  from: string;
  to: string | null;
  value: string;
  input: string;
  blockNumber: number;
  type: 'coin-transfer' | 'contract-call' | 'contract-create';
  timestamp: number;
}

function toBlockRow(b: RpcBlock): BlockRow {
  return {
    number: hexToNum(b.number),
    hash: b.hash,
    timestamp: hexToNum(b.timestamp),
    txCount: Array.isArray(b.transactions) ? b.transactions.length : 0,
    miner: b.miner ?? '',
    stream: getStreamType(b),
    gasUsed: hexToNum(b.gasUsed ?? '0x0'),
    gasLimit: hexToNum(b.gasLimit ?? '0x0'),
  };
}

function txType(tx: RpcTx): TxRow['type'] {
  if (!tx.to || tx.to === '0x0000000000000000000000000000000000000000') return 'contract-create';
  if (tx.input && tx.input !== '0x' && tx.input.length > 2) return 'contract-call';
  return 'coin-transfer';
}

export function useBlocks(pageSize = 10, streamFilter: StreamFilter = 'all') {
  const [blocks, setBlocks] = useState<BlockRow[]>([]);
  const [page, setPage] = useState(1);
  const [total, setTotal] = useState(0);
  const [loading, setLoading] = useState(true);
  const latestRef = useRef(0);

  const load = useCallback(async (pg: number) => {
    try {
      const bnHex = await rpcCall<string>('eth_blockNumber');
      const latest = hexToNum(bnHex);
      latestRef.current = latest;
      setTotal(latest + 1);

      const rows: BlockRow[] = [];
      let scanned = 0;
      const maxScan = streamFilter === 'all' ? pageSize : pageSize * 6;
      const skip = streamFilter === 'all' ? (pg - 1) * pageSize : 0;
      let skipped = 0;

      for (let n = latest; n >= 0 && rows.length < pageSize && scanned < maxScan; n--, scanned++) {
        const b = await rpcCall<RpcBlock>('eth_getBlockByNumber', [`0x${n.toString(16)}`, false]).catch(() => null);
        if (!b) continue;
        const row = toBlockRow(b);
        if (streamFilter !== 'all' && row.stream !== streamFilter) continue;
        if (skipped < skip) { skipped++; continue; }
        rows.push(row);
      }
      setBlocks(rows);
    } catch (e) {
      console.error('useBlocks error:', e);
    } finally {
      setLoading(false);
    }
  }, [pageSize, streamFilter]);

  useEffect(() => { setPage(1); }, [streamFilter]);
  useEffect(() => { setLoading(true); load(page); }, [load, page]);
  useEffect(() => {
    const id = setInterval(() => load(page), BLOCKS_REFRESH_MS);
    return () => clearInterval(id);
  }, [load, page]);

  return { blocks, page, setPage, total, loading };
}

export function useTransactions(pageSize = 10) {
  const [txs, setTxs] = useState<TxRow[]>([]);
  const [page, setPage] = useState(1);
  const [loading, setLoading] = useState(true);

  const load = useCallback(async () => {
    try {
      const bnHex = await rpcCall<string>('eth_blockNumber');
      const latest = hexToNum(bnHex);
      const rows: TxRow[] = [];
      const want = (page) * pageSize;

      for (let n = latest; n >= 0 && rows.length < want; n--) {
        const b = await rpcCall<RpcBlock>('eth_getBlockByNumber', [`0x${n.toString(16)}`, true]).catch(() => null);
        if (!b || !Array.isArray(b.transactions)) continue;
        const ts = hexToNum(b.timestamp);
        for (const tx of b.transactions as RpcTx[]) {
          rows.push({
            hash: tx.hash,
            from: tx.from,
            to: tx.to,
            value: tx.value,
            input: tx.input,
            blockNumber: hexToNum(b.number),
            type: txType(tx),
            timestamp: ts,
          });
          if (rows.length >= want) break;
        }
      }

      const start = (page - 1) * pageSize;
      setTxs(rows.slice(start, start + pageSize));
    } catch (e) {
      console.error('useTransactions error:', e);
    } finally {
      setLoading(false);
    }
  }, [page, pageSize]);

  useEffect(() => { setLoading(true); load(); }, [load]);
  useEffect(() => {
    const id = setInterval(load, BLOCKS_REFRESH_MS);
    return () => clearInterval(id);
  }, [load]);

  return { txs, page, setPage, loading };
}
