import { useState, type FormEvent } from 'react'
import { useNavigate } from 'react-router-dom'
import { search } from '@/lib/rpc'

export function SearchBar({ compact = false }: { compact?: boolean }) {
  const [query, setQuery] = useState('')
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState('')
  const navigate = useNavigate()

  async function handleSubmit(e: FormEvent) {
    e.preventDefault()
    const q = query.trim()
    if (!q) return
    setError('')
    setLoading(true)
    try {
      const result = await search(q)
      switch (result.kind) {
        case 'block':   navigate(`/block/${result.data.hash}`); break
        case 'tx':      navigate(`/tx/${result.data.hash}`); break
        case 'address': navigate(`/address/${result.address}`); break
        default:        setError('Nothing found for that query.')
      }
    } catch {
      setError('RPC error — check connection.')
    } finally {
      setLoading(false)
    }
  }

  return (
    <form onSubmit={handleSubmit} className="relative w-full">
      <input
        type="text"
        value={query}
        onChange={e => setQuery(e.target.value)}
        placeholder="Search by block hash, tx hash, address, or block number…"
        className={`w-full bg-brand-card border border-brand-border rounded-xl
                    text-gray-100 placeholder-brand-muted outline-none
                    focus:border-brand-orange transition-colors
                    ${compact ? 'py-2 pl-4 pr-24 text-sm' : 'py-3 pl-5 pr-28'}`}
      />
      <button
        type="submit"
        disabled={loading || !query.trim()}
        className="absolute right-2 top-1/2 -translate-y-1/2 btn-primary py-1.5 text-sm"
      >
        {loading ? '…' : 'Search'}
      </button>
      {error && (
        <p className="absolute left-0 -bottom-6 text-xs text-red-400">{error}</p>
      )}
    </form>
  )
}
