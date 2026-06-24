import { useState } from 'react';
import { useTransactions } from '../hooks/useBlocks';
import { fmtAddr, fmtHash, timeAgo, fmtWei } from '../lib/format';

interface Props { onTxClick: (hash: string) => void }

const TYPE_LABEL: Record<string, string> = {
  'coin-transfer': 'COIN TRANSFER',
  'contract-call': 'CONTRACT CALL',
  'contract-create': 'CONTRACT CREATE',
};

export function TransactionList({ onTxClick }: Props) {
  const [pageSize] = useState(10);
  const { txs, page, setPage, loading } = useTransactions(pageSize);

  return (
    <section className="content-card" id="transactions">
      <div className="card-header"><h2>Latest Txns</h2></div>
      <div className="card-body">
        {loading ? (
          <div className="loading-placeholder"><i className="fas fa-spinner fa-spin" /> Loading…</div>
        ) : txs.length === 0 ? (
          <div className="loading-placeholder">No transactions yet</div>
        ) : (
          <table className="data-table">
            <thead>
              <tr><th>Hash</th><th>Type</th><th>From</th><th>To</th><th>Value</th><th>Age</th></tr>
            </thead>
            <tbody>
              {txs.map(tx => (
                <tr key={tx.hash}>
                  <td>
                    <button className="link-btn" onClick={() => onTxClick(tx.hash)}>
                      {fmtHash(tx.hash, 8, 4)}
                    </button>
                  </td>
                  <td><span className={`tx-type-badge ${tx.type}`}>{TYPE_LABEL[tx.type]}</span></td>
                  <td className="text-mono text-muted">{fmtAddr(tx.from)}</td>
                  <td className="text-mono text-muted">{tx.to ? fmtAddr(tx.to) : '(create)'}</td>
                  <td>{fmtWei(tx.value)}</td>
                  <td className="text-muted">{timeAgo(tx.timestamp)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </div>
      <div className="card-footer">
        <div className="pagination">
          <button className="page-btn" onClick={() => setPage(p => Math.max(1, p - 1))} disabled={page <= 1}>← Prev</button>
          <span className="page-info">Page {page}</span>
          <button className="page-btn" onClick={() => setPage(p => p + 1)} disabled={txs.length < pageSize}>Next →</button>
        </div>
      </div>
    </section>
  );
}
