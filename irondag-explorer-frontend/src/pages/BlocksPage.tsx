import { useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { getBlockNumber, getBlockByNumber } from '@/lib/rpc'
import { BlockRow } from '@/components/BlockRow'
import type { RpcBlock } from '@/lib/types'

const PAGE_SIZE = 25

export function BlocksPage() {
  const [page, setPage] = useState(0)
  const tipQ = useQuery({ queryKey: ['blockNumber'], queryFn: getBlockNumber, refetchInterval: 8_000 })
  const tip = tipQ.data ?? 0

  const startBlock = Math.max(0, tip - page * PAGE_SIZE)
  const endBlock   = Math.max(0, tip - page * PAGE_SIZE - PAGE_SIZE + 1)

  const blocksQ = useQuery({
    queryKey: ['blockPage', tip, page],
    queryFn: async () => {
      const nums: number[] = []
      for (let n = startBlock; n >= endBlock; n--) nums.push(n)
      const results = await Promise.allSettled(nums.map(n => getBlockByNumber(n, false)))
      return results
        .filter((r): r is PromiseFulfilledResult<RpcBlock> =>
          r.status === 'fulfilled' && r.value !== null)
        .map(r => r.value)
    },
    enabled: tip > 0,
  })

  const totalPages = Math.ceil((tip + 1) / PAGE_SIZE)

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-bold">Blocks</h1>
          <p className="text-brand-muted text-sm mt-1">
            {tip > 0 ? `${(tip + 1).toLocaleString()} blocks total` : 'Loading…'}
          </p>
        </div>
        <div className="flex gap-2">
          <button
            onClick={() => setPage(p => Math.max(0, p - 1))}
            disabled={page === 0}
            className="btn-ghost"
          >
            ← Newer
          </button>
          <span className="flex items-center text-sm text-brand-muted px-2">
            {page + 1} / {totalPages || '—'}
          </span>
          <button
            onClick={() => setPage(p => p + 1)}
            disabled={page >= totalPages - 1}
            className="btn-ghost"
          >
            Older →
          </button>
        </div>
      </div>

      <div className="card">
        <div className="flex items-center gap-3 pb-3 mb-1 border-b border-brand-border
                        text-xs text-brand-muted">
          <span className="w-10">Stream</span>
          <span className="w-16">Height</span>
          <span className="flex-1">Hash</span>
          <span className="w-20 text-right">Txs</span>
          <span className="w-20 text-right">Age</span>
        </div>
        {blocksQ.isPending && (
          <div className="space-y-3 animate-pulse pt-2">
            {Array.from({ length: PAGE_SIZE }).map((_, i) => (
              <div key={i} className="h-10 bg-brand-border/40 rounded" />
            ))}
          </div>
        )}
        {blocksQ.data?.map(b => <BlockRow key={b.hash} block={b} />)}
      </div>
    </div>
  )
}
