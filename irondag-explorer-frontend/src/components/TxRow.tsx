import type { RpcTransaction } from '@/lib/types'
import { formatIdag, hexToDecimal, shortAddress } from '@/lib/utils'
import { HashBadge } from './HashBadge'

interface Props {
  tx: RpcTransaction
}

export function TxRow({ tx }: Props) {
  const value = BigInt(tx.value ?? '0x0')
  const blockNum = tx.blockNumber ? hexToDecimal(tx.blockNumber) : null

  return (
    <div className="flex items-center gap-3 py-3 border-b border-brand-border last:border-0
                    hover:bg-white/[0.02] transition-colors px-2 -mx-2 rounded">
      {/* Tx hash */}
      <div className="flex-1 min-w-0">
        <HashBadge hash={tx.hash} type="tx" chars={10} />
        {blockNum !== null && (
          <p className="text-xs text-brand-muted mt-0.5">Block #{blockNum}</p>
        )}
      </div>

      {/* From → To */}
      <div className="text-sm text-gray-400 w-48 shrink-0 hidden sm:block">
        <span className="text-gray-500">From </span>
        <HashBadge hash={tx.from} type="address" chars={5} className="!text-gray-300" />
        {tx.to && (
          <>
            <span className="text-gray-600 mx-1">→</span>
            <HashBadge hash={tx.to} type="address" chars={5} className="!text-gray-300" />
          </>
        )}
        {!tx.to && <span className="text-emerald-400 ml-1">[deploy]</span>}
      </div>

      {/* Value */}
      <span className="text-sm font-mono text-right w-32 shrink-0">
        {value > 0n ? formatIdag(value, 3) : <span className="text-brand-muted">0</span>}
      </span>
    </div>
  )
}

// Unused import guard
void shortAddress
