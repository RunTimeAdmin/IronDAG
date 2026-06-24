import { ChainStats } from '../hooks/useChainStats';
import { fmtNum } from '../lib/format';

interface Props { stats: ChainStats }

export function StatsSummary({ stats }: Props) {
  const { dagStats, blockNumber, finalizedBlock, peerCount, avgBlockTimeSec, blockRate, tps, streamA, streamB } = stats;

  const blueBlocks = dagStats?.blue_blocks ?? 0;
  const redBlocks = dagStats?.red_blocks ?? 0;
  const totalBlocks = blueBlocks + redBlocks;
  const healthPct = totalBlocks > 0 ? Math.round((blueBlocks / totalBlocks) * 100) : null;

  const totalA = typeof streamA === 'number' ? streamA : 0;
  const totalB = typeof streamB === 'number' ? streamB : 0;
  const streamTotal = totalA + totalB;

  return (
    <section className="summary-section">
      <div className="container">
        <div className="summary-grid">
          <div className="summary-card">
            <div className="summary-label">IDAG PRICE</div>
            <div className="summary-value">$0.001</div>
            <div className="summary-change positive">+0.00%</div>
          </div>

          <div className="summary-card">
            <div className="summary-label">DAG HEALTH</div>
            <div className="summary-value">{healthPct !== null ? `${healthPct}% Blue` : '--'}</div>
            <div className="health-bar">
              <div className="health-bar-blue" style={{ width: `${healthPct ?? 0}%` }} />
            </div>
          </div>

          <div className="summary-card">
            <div className="summary-label">BRAIDCORE DISTRIBUTION</div>
            <div className="summary-value">{streamTotal > 0 ? `A:${streamA} B:${streamB}` : '--'}</div>
            <div className="stream-distribution">
              <div className="stream-seg stream-seg-a" style={{ width: streamTotal > 0 ? `${(totalA / streamTotal) * 100}%` : '50%' }} />
              <div className="stream-seg stream-seg-b" style={{ width: streamTotal > 0 ? `${(totalB / streamTotal) * 100}%` : '50%' }} />
            </div>
          </div>

          <div className="summary-card">
            <div className="summary-label">TRANSACTIONS</div>
            <div className="summary-value">{dagStats ? fmtNum(dagStats.total_transactions) : '--'}</div>
            <div className="summary-subtext"><span>{tps !== null ? tps.toFixed(1) : '--'}</span> TPS</div>
          </div>

          <div className="summary-card">
            <div className="summary-label">BLOCK RATE</div>
            <div className="summary-value">{blockRate !== null ? blockRate.toFixed(2) : '--'}</div>
            <div className="summary-subtext">blocks/sec</div>
          </div>

          <div className="summary-card">
            <div className="summary-label">BLOCK HEIGHT</div>
            <div className="summary-value">{blockNumber > 0 ? fmtNum(blockNumber) : '--'}</div>
            <div className="summary-subtext">
              Tip · <span>{avgBlockTimeSec !== null ? avgBlockTimeSec.toFixed(1) : '--'}s</span> avg
            </div>
            <div className="summary-subtext" style={{ marginTop: '0.35rem', opacity: 0.9 }}>
              Finalized: <span>{finalizedBlock !== null ? fmtNum(finalizedBlock) : '--'}</span>
            </div>
          </div>

          <div className="summary-card">
            <div className="summary-label">PEER COUNT</div>
            <div className="summary-value">{fmtNum(peerCount)}</div>
            <div className="summary-subtext">connected nodes</div>
          </div>
        </div>
      </div>
    </section>
  );
}
