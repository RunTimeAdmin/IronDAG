import { useEffect, useState } from 'react';
import { rpcCall } from '../lib/rpc';
import { hexToNum, fmtNum, fmtWei, timeAgo, getStreamType, RpcBlock, RpcTx } from '../lib/format';

interface Props {
  query: string | null;
  onClose: () => void;
}

type Detail =
  | { kind: 'block'; data: RpcBlock }
  | { kind: 'tx'; data: RpcTx; timestamp: number }
  | { kind: 'error'; msg: string };

export function DetailPanel({ query, onClose }: Props) {
  const [detail, setDetail] = useState<Detail | null>(null);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    if (!query) { setDetail(null); return; }
    setLoading(true);
    setDetail(null);

    async function resolve() {
      const q = query!.trim();
      // Block hash (66 chars)
      if (/^0x[0-9a-fA-F]{64}$/.test(q)) {
        // Try tx first, then block
        try {
          const tx = await rpcCall<RpcTx>('eth_getTransactionByHash', [q]);
          if (tx) {
            let ts = 0;
            if (tx.blockHash) {
              const b = await rpcCall<RpcBlock>('eth_getBlockByHash', [tx.blockHash, false]).catch(() => null);
              if (b) ts = hexToNum(b.timestamp);
            }
            setDetail({ kind: 'tx', data: tx, timestamp: ts });
            return;
          }
        } catch { /* fall through to block */ }
        try {
          const b = await rpcCall<RpcBlock>('eth_getBlockByHash', [q, true]);
          if (b) { setDetail({ kind: 'block', data: b }); return; }
        } catch { /* */ }
      }
      // Block number
      if (/^\d+$/.test(q)) {
        try {
          const b = await rpcCall<RpcBlock>('eth_getBlockByNumber', [`0x${parseInt(q).toString(16)}`, true]);
          if (b) { setDetail({ kind: 'block', data: b }); return; }
        } catch { /* */ }
      }
      // Address — just show it
      if (/^0x[0-9a-fA-F]{40}$/.test(q)) {
        setDetail({ kind: 'error', msg: `Address lookup coming soon. Address: ${q}` });
        return;
      }
      setDetail({ kind: 'error', msg: `No results found for: ${q}` });
    }

    resolve().finally(() => setLoading(false));
  }, [query]);

  if (!query) return null;

  return (
    <section id="detail-panel" className="detail-panel">
      <div className="container">
        <div className="detail-card">
          <div className="detail-header">
            <h2 id="detail-title">
              {loading ? 'Searching…' : detail?.kind === 'block' ? `Block #${fmtNum(hexToNum(detail.data.number))}` : detail?.kind === 'tx' ? 'Transaction' : 'Result'}
            </h2>
            <button className="close-detail" onClick={onClose}><i className="fas fa-times" /></button>
          </div>
          <div className="detail-content">
            {loading && <div className="loading-placeholder"><i className="fas fa-spinner fa-spin" /> Loading…</div>}
            {!loading && detail?.kind === 'error' && <p className="text-muted">{detail.msg}</p>}
            {!loading && detail?.kind === 'block' && <BlockDetail block={detail.data} />}
            {!loading && detail?.kind === 'tx' && <TxDetail tx={detail.data} timestamp={detail.timestamp} />}
          </div>
        </div>
      </div>
    </section>
  );
}

function Row({ label, value }: { label: string; value: React.ReactNode }) {
  return (
    <div className="detail-row">
      <span className="detail-label">{label}</span>
      <span className="detail-value">{value}</span>
    </div>
  );
}

function BlockDetail({ block }: { block: RpcBlock }) {
  const bn = hexToNum(block.number);
  const ts = hexToNum(block.timestamp);
  const txs = Array.isArray(block.transactions) ? block.transactions : [];
  return (
    <div className="detail-rows">
      <Row label="Block Height" value={fmtNum(bn)} />
      <Row label="Hash" value={<span className="text-mono">{block.hash}</span>} />
      <Row label="Timestamp" value={`${new Date(ts * 1000).toLocaleString()} (${timeAgo(ts)})`} />
      <Row label="Stream" value={<span className={`stream-badge stream-${getStreamType(block).toLowerCase()}`}>Stream {getStreamType(block)}</span>} />
      <Row label="Transactions" value={txs.length} />
      <Row label="Miner" value={<span className="text-mono">{block.miner ?? '--'}</span>} />
      <Row label="Gas Used" value={fmtNum(hexToNum(block.gasUsed ?? '0x0'))} />
      <Row label="Parent Hash" value={<span className="text-mono">{block.parentHash ?? '--'}</span>} />
      {txs.length > 0 && (
        <div style={{ marginTop: '1rem' }}>
          <h3 style={{ marginBottom: '0.5rem', color: 'var(--text-secondary)' }}>Transactions ({txs.length})</h3>
          {(txs as RpcTx[]).map(tx => (
            <div key={tx.hash} className="detail-tx-row text-mono text-muted" style={{ fontSize: '0.8rem', padding: '0.25rem 0' }}>
              {tx.hash}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

function TxDetail({ tx, timestamp }: { tx: RpcTx; timestamp: number }) {
  return (
    <div className="detail-rows">
      <Row label="Hash" value={<span className="text-mono">{tx.hash}</span>} />
      <Row label="Block" value={tx.blockNumber ? fmtNum(hexToNum(tx.blockNumber)) : 'Pending'} />
      {timestamp > 0 && <Row label="Timestamp" value={`${new Date(timestamp * 1000).toLocaleString()} (${timeAgo(timestamp)})`} />}
      <Row label="From" value={<span className="text-mono">{tx.from}</span>} />
      <Row label="To" value={<span className="text-mono">{tx.to ?? '(contract create)'}</span>} />
      <Row label="Value" value={fmtWei(tx.value)} />
      <Row label="Gas Limit" value={tx.gas ? fmtNum(hexToNum(tx.gas)) : '--'} />
      <Row label="Gas Price" value={tx.gasPrice ? fmtWei(tx.gasPrice) : '--'} />
      <Row label="Nonce" value={tx.nonce ? hexToNum(tx.nonce) : '--'} />
    </div>
  );
}
