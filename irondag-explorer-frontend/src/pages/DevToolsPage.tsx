import { useState, type FormEvent } from 'react'
import { useQuery } from '@tanstack/react-query'
import { getBlockNumber, getChainId, getPeerCount, getDagStats, RPC_URL } from '@/lib/rpc'

export function DevToolsPage() {
  const [method, setMethod] = useState('eth_blockNumber')
  const [params, setParams] = useState('[]')
  const [response, setResponse] = useState('')
  const [loading, setLoading] = useState(false)

  const infoQ = useQuery({
    queryKey: ['devInfo'],
    queryFn: async () => ({
      blockNumber: await getBlockNumber(),
      chainId: await getChainId(),
      peers: await getPeerCount(),
      dagStats: await getDagStats(),
    }),
    refetchInterval: 30_000,
  })

  async function callRpc(e: FormEvent) {
    e.preventDefault()
    setLoading(true)
    try {
      let parsedParams: unknown[] = []
      try { parsedParams = JSON.parse(params) } catch { parsedParams = [] }

      const res = await fetch(RPC_URL, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ jsonrpc: '2.0', id: 1, method, params: parsedParams }),
      })
      const json = await res.json()
      setResponse(JSON.stringify(json, null, 2))
    } catch (err) {
      setResponse(`Error: ${(err as Error).message}`)
    } finally {
      setLoading(false)
    }
  }

  const PRESETS = [
    { label: 'Block number', method: 'eth_blockNumber', params: '[]' },
    { label: 'Chain ID', method: 'eth_chainId', params: '[]' },
    { label: 'Peer count', method: 'net_peerCount', params: '[]' },
    { label: 'DAG stats', method: 'idag_getDagStats', params: '[]' },
    { label: 'Latest block', method: 'eth_getBlockByNumber', params: '["latest", false]' },
    { label: 'Stream A blocks', method: 'idag_getBlocksByStream', params: '["StreamA", 5]' },
    { label: 'Stream B blocks', method: 'idag_getBlocksByStream', params: '["StreamB", 5]' },
  ]

  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-2xl font-bold">Dev Tools</h1>
        <p className="text-brand-muted text-sm mt-1">
          RPC playground and network diagnostics for IronDAG testnet.
        </p>
      </div>

      {/* Network info */}
      <div className="grid grid-cols-2 md:grid-cols-4 gap-4">
        {[
          { label: 'Chain ID',      value: infoQ.data?.chainId ?? '—' },
          { label: 'Block Height',  value: infoQ.data?.blockNumber?.toLocaleString() ?? '—' },
          { label: 'Peers',         value: infoQ.data?.peers ?? '—' },
          { label: 'RPC Endpoint',  value: RPC_URL },
        ].map(({ label, value }) => (
          <div key={label} className="card">
            <span className="label">{label}</span>
            <p className="text-white font-mono text-sm mt-1 truncate">{value}</p>
          </div>
        ))}
      </div>

      {/* MetaMask config */}
      <div className="card space-y-3">
        <h2 className="font-semibold text-white">Add to MetaMask</h2>
        <div className="grid sm:grid-cols-2 gap-3 text-sm">
          {[
            ['Network Name', 'IronDAG Testnet'],
            ['Chain ID',     '11567'],
            ['Symbol',       'IDAG'],
            ['RPC URL',      'https://explorer.irondag.io/rpc'],
          ].map(([k, v]) => (
            <div key={k} className="flex gap-2">
              <span className="text-brand-muted w-32 shrink-0">{k}:</span>
              <span className="font-mono text-white">{v}</span>
            </div>
          ))}
        </div>
      </div>

      {/* RPC playground */}
      <div className="card space-y-4">
        <h2 className="font-semibold text-white">RPC Playground</h2>

        {/* Presets */}
        <div className="flex flex-wrap gap-2">
          {PRESETS.map(p => (
            <button
              key={p.label}
              type="button"
              onClick={() => { setMethod(p.method); setParams(p.params) }}
              className="btn-ghost text-xs"
            >
              {p.label}
            </button>
          ))}
        </div>

        <form onSubmit={callRpc} className="space-y-3">
          <div className="space-y-1">
            <label className="label">Method</label>
            <input
              type="text"
              value={method}
              onChange={e => setMethod(e.target.value)}
              className="w-full bg-brand-dark border border-brand-border rounded-lg px-4 py-2
                         font-mono text-sm text-gray-100 outline-none focus:border-brand-orange
                         transition-colors"
            />
          </div>
          <div className="space-y-1">
            <label className="label">Params (JSON array)</label>
            <input
              type="text"
              value={params}
              onChange={e => setParams(e.target.value)}
              className="w-full bg-brand-dark border border-brand-border rounded-lg px-4 py-2
                         font-mono text-sm text-gray-100 outline-none focus:border-brand-orange
                         transition-colors"
            />
          </div>
          <button type="submit" disabled={loading} className="btn-primary">
            {loading ? 'Calling…' : 'Call RPC'}
          </button>
        </form>

        {response && (
          <div className="space-y-1">
            <span className="label">Response</span>
            <pre className="bg-brand-dark border border-brand-border rounded-lg p-4 text-xs
                            text-gray-300 overflow-x-auto font-mono whitespace-pre-wrap">
              {response}
            </pre>
          </div>
        )}
      </div>
    </div>
  )
}
