import { Link } from 'react-router-dom'
import { shortHash, copyToClipboard } from '@/lib/utils'

interface Props {
  hash: string
  type?: 'block' | 'tx' | 'address'
  chars?: number
  className?: string
  copyable?: boolean
}

export function HashBadge({ hash, type, chars = 8, className = '', copyable = true }: Props) {
  const label = shortHash(hash, chars)

  const inner = (
    <span
      className={`hash text-sm ${className}`}
      onClick={copyable ? (e) => { e.preventDefault(); copyToClipboard(hash) } : undefined}
      title={copyable ? `${hash} (click to copy)` : hash}
    >
      {label}
    </span>
  )

  if (type === 'block')   return <Link to={`/block/${hash}`}>{inner}</Link>
  if (type === 'tx')      return <Link to={`/tx/${hash}`}>{inner}</Link>
  if (type === 'address') return <Link to={`/address/${hash}`}>{inner}</Link>
  return inner
}
