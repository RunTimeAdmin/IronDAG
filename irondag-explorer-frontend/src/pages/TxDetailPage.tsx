import { useParams, Link } from 'react-router-dom'
import { useQuery } from '@tanstack/react-query'
import { getTransactionByHash, getTransactionReceipt } from '@/lib/rpc'
import { HashBadge } from '@/components/HashBadge'
import { formatIdag, hexToDecimal } from '@/lib/utils'

export function TxDetailPage() {
  const { hash } = useParams<{ hash: string }>()

  const txQ = useQuery({
    queryKey: ['tx', hash],
    queryFn: () => getTransactionByHash(hash!),
    enabled: !!hash,
  })

  const receiptQ = useQuery({
    queryKey: ['receipt', hash],
    queryFn: () => getTransactionReceipt(hash!),
    enabled: !!hash,
  })

  const tx = txQ.data
  const receipt = receiptQ.data

  if (txQ.isPending) return <Loading />
  if (!tx) return (
    <div className="card text-center py-16 space-y-3">
      <p className="text-xl text-gray-400">Transaction not found</p>
      <p className="text-sm text-brand-muted">It may still be pending or the hash is incorrect.</p>
    </div>
  )

  const value = BigInt(tx.value ?? '0x0')
  const gasPrice = BigInt(tx.gasPrice ?? '0x0')
  const gas = hexToDecimal(tx.gas)
  const nonce = hexToDecimal(tx.nonce)
  const blockNum = tx.blockNumber ? hexToDecimal(tx.blockNumber) : null
  const status = receipt ? (receipt.status === '0x1' ? 'Success' : 'Failed') : 'Pending'

  const rows: [string, React.ReactNode][] = [
    ['Status', <StatusBadge status={status} />],
    ['Hash', <HashBadge hash={tx.hash} copyable />],
    ...(blockNum !== null ? [['Block', <Link to={`/block/${tx.blockHash!}`} className="text-brand-orange hover:text-orange-400 font-semibold">#{blockNum}</Link>] as [string, React.ReactNode]] : []),
    ['From', <HashBadge hash={tx.from} type="address" chars={20} />],
    ['To', tx.to
      ? <HashBadge hash={tx.to} type="address" chars={20} />
      : <span className="text-emerald-400 text-sm">Contract Creation</span>
    ],
    ['Value', formatIdag(value)],
    ['Gas Limit', gas.toLocaleString()],
    ['Gas Price', `${formatIdag(gasPrice, 6)} / gas`],
    ['Nonce', nonce.toString()],
    ['Input Data', tx.input && tx.input !== '0x'
      ? <details className="text-xs"><summary className="cursor-pointer text-brand-muted">{((tx.input.length - 2) / 2).toLocaleString()} bytes</summary><p className="font-mono mt-2 break-all text-gray-400">{tx.input}</p></details>
      : <span className="text-brand-muted">—</span>
    ],
  ]

  return (
    <div className="space-y-6">
      <div className="flex items-center gap-4">
        <h1 className="text-2xl font-bold">Transaction</h1>
      </div>
      <div className="card divide-y divide-brand-border">
        {rows.map(([label, value]) => (
          <div key={label} className="flex items-start gap-4 py-3 first:pt-0 last:pb-0">
            <span className="label w-28 shrink-0 pt-0.5">{label}</span>
            <div className="flex-1 min-w-0 text-sm">{value}</div>
          </div>
        ))}
      </div>
    </div>
  )
}

function StatusBadge({ status }: { status: string }) {
  const cls =
    status === 'Success' ? 'bg-emerald-900/40 text-emerald-300 border-emerald-700/50' :
    status === 'Failed'  ? 'bg-red-900/40 text-red-300 border-red-700/50' :
                           'bg-yellow-900/40 text-yellow-300 border-yellow-700/50'
  return (
    <span className={`text-xs font-semibold px-2 py-0.5 rounded border ${cls}`}>
      {status}
    </span>
  )
}

function Loading() {
  return (
    <div className="card animate-pulse space-y-4">
      {Array.from({ length: 8 }).map((_, i) => (
        <div key={i} className="h-8 bg-brand-border/40 rounded" />
      ))}
    </div>
  )
}
