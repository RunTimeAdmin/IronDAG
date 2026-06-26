import { useState, type FormEvent } from 'react'
import { requestFaucet } from '@/lib/rpc'
import { HashBadge } from '@/components/HashBadge'
import type { FaucetResult } from '@/lib/types'

export function FaucetPage() {
  const [address, setAddress] = useState('')
  const [loading, setLoading] = useState(false)
  const [result, setResult] = useState<FaucetResult | null>(null)
  const [error, setError] = useState('')

  async function handleSubmit(e: FormEvent) {
    e.preventDefault()
    const addr = address.trim()
    if (!addr) return
    if (!/^0x[0-9a-fA-F]{40}$/.test(addr)) {
      setError('Invalid address format (must be 0x + 40 hex chars)')
      return
    }
    setError('')
    setResult(null)
    setLoading(true)
    try {
      const res = await requestFaucet(addr)
      setResult(res)
    } catch (err) {
      setError((err as Error).message ?? 'Faucet request failed')
    } finally {
      setLoading(false)
    }
  }

  return (
    <div className="max-w-lg mx-auto space-y-6">
      <div>
        <h1 className="text-2xl font-bold">Testnet Faucet</h1>
        <p className="text-brand-muted text-sm mt-1">
          Request testnet IDAG tokens for development and testing.
        </p>
      </div>

      {/* Info card */}
      <div className="card border-brand-orange/30 bg-brand-orange/5 text-sm space-y-1">
        <p className="font-semibold text-brand-orange">Testnet tokens only</p>
        <p className="text-gray-400">
          These tokens have no monetary value. Each request delivers a fixed amount to your address.
          Rate limits apply per address.
        </p>
      </div>

      {/* Form */}
      <form onSubmit={handleSubmit} className="card space-y-4">
        <div className="space-y-1">
          <label className="label">Recipient Address</label>
          <input
            type="text"
            value={address}
            onChange={e => setAddress(e.target.value)}
            placeholder="0x..."
            className="w-full bg-brand-dark border border-brand-border rounded-lg px-4 py-3
                       font-mono text-sm text-gray-100 placeholder-brand-muted outline-none
                       focus:border-brand-orange transition-colors"
          />
        </div>
        {error && <p className="text-sm text-red-400">{error}</p>}
        <button type="submit" disabled={loading || !address.trim()} className="btn-primary w-full">
          {loading ? 'Requesting…' : 'Request Tokens'}
        </button>
      </form>

      {/* Result */}
      {result && (
        <div className={`card ${result.success ? 'border-emerald-700/50' : 'border-red-700/50'}`}>
          <p className={`font-semibold mb-2 ${result.success ? 'text-emerald-400' : 'text-red-400'}`}>
            {result.success ? 'Tokens Sent!' : 'Request Failed'}
          </p>
          {result.message && (
            <p className="text-sm text-gray-400 mb-2">{result.message}</p>
          )}
          {result.tx_hash && (
            <div className="space-y-1">
              <span className="label">Transaction</span>
              <HashBadge hash={result.tx_hash} type="tx" chars={16} />
            </div>
          )}
        </div>
      )}
    </div>
  )
}
