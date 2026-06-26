import { useQuery } from '@tanstack/react-query'
import { Link } from 'react-router-dom'
import { getBlockNumber, getPeerCount, getDagStats, getTps, getRecentBlocks, getBlocksByStream } from '@/lib/rpc'
import { StatCard } from '@/components/StatCard'
import { BlockRow } from '@/components/BlockRow'
import { TxRow } from '@/components/TxRow'
import { IconBlock, IconNetwork, IconDag, IconZap } from '@/components/icons'
import type { RpcTransaction } from '@/lib/types'

// Block height polls fast (1 cheap call). Everything else only refetches when
// the height changes — avoids hammering the RPC with 20+ calls per cycle.
const POLL_HEIGHT  = 8_000   // how often to check for new blocks
const REFETCH_SLOW = 30_000  // slow-changing data (peers, DAG stats, TPS)

export function HomePage() {
  const blockNum = useQuery({ queryKey: ['blockNumber'], queryFn: getBlockNumber, refetchInterval: POLL_HEIGHT })
  const tip = blockNum.data ?? 0

  const peers    = useQuery({ queryKey: ['peerCount'],   queryFn: getPeerCount,   refetchInterval: REFETCH_SLOW })
  const dag      = useQuery({ queryKey: ['dagStats'],    queryFn: getDagStats,    refetchInterval: REFETCH_SLOW })
  const tpsQuery = useQuery({ queryKey: ['tps'],         queryFn: () => getTps(10), refetchInterval: REFETCH_SLOW })

  // Include tip in queryKey so these re-run automatically when a new block arrives
  const blocks   = useQuery({ queryKey: ['recentBlocks', tip], queryFn: () => getRecentBlocks(20), enabled: tip > 0 })
  const streamA  = useQuery({ queryKey: ['streamA', tip],      queryFn: () => getBlocksByStream('A', 5), enabled: tip > 0 })
  const streamB  = useQuery({ queryKey: ['streamB', tip],      queryFn: () => getBlocksByStream('B', 5), enabled: tip > 0 })

  // Collect recent txs from recent blocks
  const recentTxs: RpcTransaction[] = []
  if (blocks.data) {
    for (const b of blocks.data) {
      if (Array.isArray(b.transactions)) {
        for (const tx of b.transactions) {
          if (typeof tx === 'object' && 'hash' in tx) recentTxs.push(tx as RpcTransaction)
          if (recentTxs.length >= 10) break
        }
      }
      if (recentTxs.length >= 10) break
    }
  }

  const health = (dag.data?.blue_blocks != null && dag.data?.red_blocks != null)
    ? `${((dag.data.blue_blocks / (dag.data.blue_blocks + dag.data.red_blocks)) * 100).toFixed(1)}%`
    : '—'
  const tps = tpsQuery.data != null ? tpsQuery.data.toFixed(2) : '—'

  return (
    <div className="space-y-8">
      {/* Hero */}
      <div className="text-center space-y-2 py-4">
        <h1 className="text-3xl font-bold">
          Iron<span className="text-brand-orange">DAG</span> Explorer
        </h1>
        <p className="text-brand-muted">Post-Quantum BraidCore BlockDAG · Testnet · Chain ID 11567</p>
      </div>

      {/* Stats */}
      <div className="grid grid-cols-2 lg:grid-cols-4 gap-4">
        <StatCard
          label="Block Height"
          value={blockNum.data?.toLocaleString() ?? '—'}
          icon={<IconBlock className="w-4 h-4" />}
          accent
        />
        <StatCard
          label="Peers"
          value={peers.data ?? '—'}
          icon={<IconNetwork className="w-4 h-4" />}
        />
        <StatCard
          label="DAG Health"
          value={health}
          sub={`${dag.data?.blue_blocks ?? '—'} blue / ${dag.data?.red_blocks ?? '—'} red`}
          icon={<IconDag className="w-4 h-4" />}
        />
        <StatCard
          label="TPS"
          value={tps}
          sub={`${dag.data?.total_transactions?.toLocaleString() ?? '—'} total txs`}
          icon={<IconZap className="w-4 h-4" />}
        />
      </div>

      {/* Stream pills */}
      <div className="grid grid-cols-2 gap-4">
        {[
          { label: 'Stream A · Blake3 PoW', data: streamA.data, cls: 'stream-a' },
          { label: 'Stream B · B3MemHash PoW', data: streamB.data, cls: 'stream-b' },
        ].map(({ label, data, cls }) => (
          <div key={label} className="card space-y-2">
            <div className="flex items-center justify-between">
              <span className={`text-xs px-2 py-0.5 rounded font-semibold ${cls}`}>{label}</span>
              <span className="text-xs text-brand-muted">{data?.length ?? 0} recent</span>
            </div>
            {data?.slice(0, 3).map(b => (
              <div key={b.hash} className="flex items-center gap-2 text-sm">
                <Link to={`/block/${b.hash}`} className="text-brand-orange hover:text-orange-400 font-mono">
                  #{b.number}
                </Link>
                <span className="text-brand-muted text-xs">{b.tx_count ?? 0} txs</span>
              </div>
            ))}
            {!data && <p className="text-xs text-brand-muted">Loading…</p>}
          </div>
        ))}
      </div>

      {/* Recent blocks + txs */}
      <div className="grid lg:grid-cols-2 gap-6">
        {/* Blocks */}
        <div className="card space-y-1">
          <div className="flex items-center justify-between mb-3">
            <h2 className="font-semibold text-white">Latest Blocks</h2>
            <Link to="/blocks" className="btn-ghost text-xs">View all →</Link>
          </div>
          {blocks.isPending && <Skeleton rows={8} />}
          {blocks.data?.slice(0, 10).map(b => <BlockRow key={b.hash} block={b} />)}
        </div>

        {/* Transactions */}
        <div className="card space-y-1">
          <div className="flex items-center justify-between mb-3">
            <h2 className="font-semibold text-white">Latest Transactions</h2>
          </div>
          {blocks.isPending && <Skeleton rows={8} />}
          {recentTxs.length === 0 && !blocks.isPending && (
            <p className="text-sm text-brand-muted py-4 text-center">No transactions yet</p>
          )}
          {recentTxs.map(tx => <TxRow key={tx.hash} tx={tx} />)}
        </div>
      </div>
    </div>
  )
}

function Skeleton({ rows }: { rows: number }) {
  return (
    <div className="space-y-3 animate-pulse">
      {Array.from({ length: rows }).map((_, i) => (
        <div key={i} className="h-10 bg-brand-border/40 rounded" />
      ))}
    </div>
  )
}
