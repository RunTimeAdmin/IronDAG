import { Link } from 'react-router-dom'
import type { RpcBlock } from '@/lib/types'
import { hexToDecimal, timeAgo, streamClass, streamLabel } from '@/lib/utils'
import { HashBadge } from './HashBadge'

interface Props {
  block: RpcBlock
}

export function BlockRow({ block }: Props) {
  const num = hexToDecimal(block.number)
  const ts = hexToDecimal(block.timestamp)
  const txCount = Array.isArray(block.transactions) ? block.transactions.length : 0
  const stream = block.streamType ?? block.stream

  return (
    <div className="flex items-center gap-3 py-3 border-b border-brand-border last:border-0
                    hover:bg-white/[0.02] transition-colors px-2 -mx-2 rounded">
      {/* Stream badge */}
      <span className={`text-xs font-mono px-2 py-0.5 rounded font-semibold min-w-[2.5rem]
                        text-center ${streamClass(stream)}`}>
        {streamLabel(stream)}
      </span>

      {/* Block number */}
      <Link
        to={`/block/${block.hash}`}
        className="text-brand-orange hover:text-orange-400 font-semibold w-16 shrink-0"
      >
        #{num}
      </Link>

      {/* Hash */}
      <div className="flex-1 min-w-0">
        <HashBadge hash={block.hash} type="block" chars={10} />
      </div>

      {/* Tx count */}
      <span className="text-sm text-gray-400 w-20 text-right shrink-0">
        {txCount} tx{txCount !== 1 ? 's' : ''}
      </span>

      {/* Age */}
      <span className="text-sm text-brand-muted w-20 text-right shrink-0">
        {ts > 0 ? timeAgo(ts) : '—'}
      </span>
    </div>
  )
}
