import { useState } from 'react';
import { useBlocks, StreamFilter } from '../hooks/useBlocks';
import { fmtNum, fmtHash, fmtAddr, timeAgo } from '../lib/format';

interface Props { onBlockClick: (hash: string) => void }

export function BlockList({ onBlockClick }: Props) {
  const [streamFilter, setStreamFilter] = useState<StreamFilter>('all');
  const [pageSize, setPageSize] = useState(10);
  const { blocks, page, setPage, total, loading } = useBlocks(pageSize, streamFilter);

  const start = (page - 1) * pageSize + 1;
  const end = Math.min(page * pageSize, total);

  return (
    <section className="content-card" id="blocks">
      <div className="card-header"><h2>Latest Blocks</h2></div>
      <div className="stream-filter-tabs">
        {(['all', 'A', 'B'] as StreamFilter[]).map(f => (
          <button key={f} className={`stream-tab${streamFilter === f ? ' active' : ''}`} onClick={() => { setStreamFilter(f); setPage(1); }}>
            {f === 'all' ? 'All Streams' : `Stream ${f}`}
          </button>
        ))}
      </div>
      <div className="card-body">
        {loading ? (
          <div className="loading-placeholder"><i className="fas fa-spinner fa-spin" /> Loading…</div>
        ) : blocks.length === 0 ? (
          <div className="loading-placeholder">No blocks found</div>
        ) : (
          <table className="data-table">
            <thead>
              <tr><th>Block</th><th>Stream</th><th>Age</th><th>Txns</th><th>Miner</th></tr>
            </thead>
            <tbody>
              {blocks.map(b => (
                <tr key={b.hash}>
                  <td>
                    <button className="link-btn" onClick={() => onBlockClick(b.hash)}>
                      #{fmtNum(b.number)}
                    </button>
                  </td>
                  <td><span className={`stream-badge stream-${b.stream.toLowerCase()}`}>Stream {b.stream}</span></td>
                  <td className="text-muted" data-timestamp={b.timestamp}>{timeAgo(b.timestamp)}</td>
                  <td>{b.txCount}</td>
                  <td className="text-mono text-muted">{fmtHash(b.hash, 8, 4)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </div>
      <div className="card-footer">
        <div className="pagination">
          <button className="page-btn" onClick={() => setPage(p => Math.max(1, p - 1))} disabled={page <= 1}>← Prev</button>
          <span className="page-info">{total > 0 ? `${start}–${end} of ${fmtNum(total)}` : `Page ${page}`}</span>
          <button className="page-btn" onClick={() => setPage(p => p + 1)} disabled={end >= total}>Next →</button>
          <select className="page-size" value={pageSize} onChange={e => { setPageSize(parseInt(e.target.value)); setPage(1); }}>
            <option value={10}>10</option><option value={25}>25</option><option value={50}>50</option>
          </select>
        </div>
      </div>
    </section>
  );
}
