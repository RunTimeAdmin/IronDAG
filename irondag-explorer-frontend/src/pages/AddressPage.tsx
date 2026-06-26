import { useParams } from 'react-router-dom'
import { useQuery } from '@tanstack/react-query'
import { getBalance, getNonce } from '@/lib/rpc'
import { HashBadge } from '@/components/HashBadge'
import { IconCopy } from '@/components/icons'
import { formatIdag, copyToClipboard } from '@/lib/utils'

export function AddressPage() {
  const { address } = useParams<{ address: string }>()

  const balanceQ = useQuery({
    queryKey: ['balance', address],
    queryFn: () => getBalance(address!),
    enabled: !!address,
    refetchInterval: 15_000,
  })

  const nonceQ = useQuery({
    queryKey: ['nonce', address],
    queryFn: () => getNonce(address!),
    enabled: !!address,
  })

  if (!address) return null

  const rows: [string, React.ReactNode][] = [
    ['Address', (
      <span className="flex items-center gap-2">
        <span className="font-mono text-white break-all">{address}</span>
        <button
          onClick={() => copyToClipboard(address)}
          className="text-brand-muted hover:text-white transition-colors shrink-0"
          title="Copy address"
        >
          <IconCopy />
        </button>
      </span>
    )],
    ['Balance', balanceQ.isPending
      ? <span className="animate-pulse bg-brand-border/40 h-5 w-32 rounded inline-block" />
      : <span className="text-white font-semibold">{formatIdag(balanceQ.data ?? 0n)}</span>
    ],
    ['Nonce', nonceQ.isPending
      ? <span className="animate-pulse bg-brand-border/40 h-5 w-16 rounded inline-block" />
      : <span className="text-white">{nonceQ.data ?? '—'}</span>
    ],
  ]

  return (
    <div className="space-y-6">
      <div className="flex items-center gap-4">
        <h1 className="text-2xl font-bold">Address</h1>
        <HashBadge hash={address} chars={12} className="text-base" />
      </div>

      <div className="card divide-y divide-brand-border">
        {rows.map(([label, value]) => (
          <div key={label} className="flex items-start gap-4 py-3 first:pt-0 last:pb-0">
            <span className="label w-24 shrink-0 pt-0.5">{label}</span>
            <div className="flex-1 min-w-0 text-sm">{value}</div>
          </div>
        ))}
      </div>

      <div className="card text-center py-10 text-brand-muted text-sm">
        Transaction history requires an indexer not yet deployed on this testnet.
      </div>
    </div>
  )
}
