import { useParams, Link } from 'react-router-dom'
import { useQuery } from '@tanstack/react-query'
import { getBlockByHash, getBlockByNumber } from '@/lib/rpc'
import { HashBadge } from '@/components/HashBadge'
import { TxRow } from '@/components/TxRow'
import { hexToDecimal, timeAgo, formatTimestamp, streamLabel, streamClass } from '@/lib/utils'
import type { RpcTransaction } from '@/lib/types'

export function BlockDetailPage() {
  const { id } = useParams<{ id: string }>()

  const blockQ = useQuery({
    queryKey: ['block', id],
    queryFn: () => {
      if (!id) return null
      if (id.startsWith('0x')) return getBlockByHash(id)
      return getBlockByNumber(parseInt(id, 10))
    },
    enabled: !!id,
  })

  const block = blockQ.data

  if (blockQ.isPending) return <LoadingState />
  if (blockQ.isError || !block) return (
    <div className="card text-center py-16 space-y-3">
      <p className="text-xl text-gray-400">Block not found</p>
      <Link to="/blocks" className="btn-ghost inline-block">← Back to blocks</Link>
    </div>
  )

  const num = hexToDecimal(block.number)
  const ts  = hexToDecimal(block.timestamp)
  const gasUsed = hexToDecimal(block.gasUsed)
  const gasLimit = hexToDecimal(block.gasLimit)
  const difficulty = hexToDecimal(block.difficulty)
  const stream = block.streamType ?? block.stream
  const txs = (block.transactions ?? []) as RpcTransaction[]

  const rows: [string, React.ReactNode][] = [
    ['Block Height', <span className="text-white font-semibold">#{num}</span>],
    ['Stream', <span className={`text-xs px-2 py-0.5 rounded font-semibold ${streamClass(stream)}`}>{streamLabel(stream) === '?' ? 'Unknown' : `Stream ${streamLabel(stream)}`}</span>],
    ['Timestamp', <span className="text-white">{formatTimestamp(ts)} <span className="text-brand-muted">({timeAgo(ts)})</span></span>],
    ['Hash', <HashBadge hash={block.hash} copyable />],
    ['Parent', Array.isArray(block.parentHash)
      ? <span>{block.parentHash.length} parents</span>
      : <HashBadge hash={block.parentHash as string} type="block" />
    ],
    ['Miner', <HashBadge hash={block.miner} type="address" chars={20} />],
    ['Difficulty', difficulty.toLocaleString()],
    ['Nonce', `0x${hexToDecimal(block.nonce).toString(16)}`],
    ['Gas Used', `${gasUsed.toLocaleString()} / ${gasLimit.toLocaleString()}`],
    ['Transactions', `${txs.length}`],
  ]

  return (
    <div className="space-y-6">
      <div className="flex items-center gap-4">
        <Link to="/blocks" className="btn-ghost text-sm">← Blocks</Link>
        <h1 className="text-2xl font-bold">Block <span className="text-brand-orange">#{num}</span></h1>
      </div>

      {/* Detail table */}
      <div className="card divide-y divide-brand-border">
        {rows.map(([label, value]) => (
          <div key={label} className="flex items-start gap-4 py-3 first:pt-0 last:pb-0">
            <span className="label w-36 shrink-0 pt-0.5">{label}</span>
            <div className="flex-1 min-w-0 text-sm">{value}</div>
          </div>
        ))}
      </div>

      {/* Transactions */}
      {txs.length > 0 && (
        <div className="card space-y-1">
          <h2 className="font-semibold text-white mb-3">
            Transactions <span className="text-brand-muted font-normal">({txs.length})</span>
          </h2>
          {txs.map(tx => <TxRow key={tx.hash} tx={tx} />)}
        </div>
      )}
    </div>
  )
}

function LoadingState() {
  return (
    <div className="space-y-6 animate-pulse">
      <div className="h-8 w-48 bg-brand-border/40 rounded" />
      <div className="card space-y-4">
        {Array.from({ length: 10 }).map((_, i) => (
          <div key={i} className="h-8 bg-brand-border/40 rounded" />
        ))}
      </div>
    </div>
  )
}
