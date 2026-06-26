import type { RpcBlock, RpcTransaction, DagStats, StreamBlock, FaucetResult } from './types'

// Resolve RPC endpoint: nginx proxies /rpc on the production host
const isExplorerHost =
  typeof window !== 'undefined' &&
  (window.location.hostname === 'explorer.irondag.io' ||
    window.location.hostname === 'localhost')

export const RPC_URL = isExplorerHost ? '/rpc' : 'http://localhost:8546'

let _idSeq = 1

async function call<T>(method: string, params: unknown[] = []): Promise<T> {
  const res = await fetch(RPC_URL, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ jsonrpc: '2.0', id: _idSeq++, method, params }),
    signal: AbortSignal.timeout(10_000),
  })
  if (!res.ok) throw new Error(`HTTP ${res.status}`)
  const json = await res.json()
  if (json.error) throw new Error(json.error.message ?? JSON.stringify(json.error))
  return json.result as T
}

// ── Chain ────────────────────────────────────────────────────────────────────

export async function getBlockNumber(): Promise<number> {
  const hex = await call<string>('eth_blockNumber')
  return parseInt(hex, 16)
}

export async function getChainId(): Promise<number> {
  const hex = await call<string>('eth_chainId')
  return parseInt(hex, 16)
}

export async function getPeerCount(): Promise<number> {
  const hex = await call<string>('net_peerCount')
  return parseInt(hex, 16)
}

// ── Blocks ────────────────────────────────────────────────────────────────────

export async function getBlockByNumber(
  n: number | 'latest',
  fullTxs = true,
): Promise<RpcBlock | null> {
  const tag = n === 'latest' ? 'latest' : `0x${n.toString(16)}`
  return call<RpcBlock | null>('eth_getBlockByNumber', [tag, fullTxs])
}

export async function getBlockByHash(hash: string, fullTxs = true): Promise<RpcBlock | null> {
  return call<RpcBlock | null>('eth_getBlockByHash', [hash, fullTxs])
}

export async function getRecentBlocks(count = 20): Promise<RpcBlock[]> {
  const tip = await getBlockNumber()
  const promises: Promise<RpcBlock | null>[] = []
  for (let i = tip; i > Math.max(0, tip - count); i--) {
    promises.push(getBlockByNumber(i, false))
  }
  const results = await Promise.allSettled(promises)
  return results
    .filter((r): r is PromiseFulfilledResult<RpcBlock> => r.status === 'fulfilled' && r.value !== null)
    .map(r => r.value)
}

export async function getBlocksByStream(stream: string, count = 20): Promise<StreamBlock[]> {
  try {
    return await call<StreamBlock[]>('irondag_getBlocksByStream', [stream, count])
  } catch {
    return []
  }
}

// ── Transactions ──────────────────────────────────────────────────────────────

export async function getTransactionByHash(hash: string): Promise<RpcTransaction | null> {
  return call<RpcTransaction | null>('eth_getTransactionByHash', [hash])
}

export async function getTransactionReceipt(hash: string): Promise<Record<string, unknown> | null> {
  return call<Record<string, unknown> | null>('eth_getTransactionReceipt', [hash])
}

// ── Address ───────────────────────────────────────────────────────────────────

export async function getBalance(address: string): Promise<bigint> {
  const hex = await call<string>('eth_getBalance', [address, 'latest'])
  return BigInt(hex)
}

export async function getNonce(address: string): Promise<number> {
  const hex = await call<string>('eth_getTransactionCount', [address, 'latest'])
  return parseInt(hex, 16)
}

// ── DAG stats ─────────────────────────────────────────────────────────────────

export async function getDagStats(): Promise<DagStats> {
  try {
    return await call<DagStats>('irondag_getDagStats', [])
  } catch {
    return {}
  }
}

export async function getTps(windowSecs = 10): Promise<number> {
  try {
    const s = await call<string>('irondag_getTps', [windowSecs])
    return parseFloat(s)
  } catch {
    return 0
  }
}

// ── Faucet ────────────────────────────────────────────────────────────────────

export async function requestFaucet(address: string): Promise<FaucetResult> {
  return call<FaucetResult>('irondag_faucet', [address])
}

// ── Search ────────────────────────────────────────────────────────────────────

export type SearchResult =
  | { kind: 'block'; data: RpcBlock }
  | { kind: 'tx'; data: RpcTransaction }
  | { kind: 'address'; address: string }
  | { kind: 'notfound' }

export async function search(query: string): Promise<SearchResult> {
  const q = query.trim()

  // 66-char hex = tx or block hash
  if (/^0x[0-9a-fA-F]{64}$/.test(q)) {
    const [block, tx] = await Promise.allSettled([
      getBlockByHash(q),
      getTransactionByHash(q),
    ])
    if (tx.status === 'fulfilled' && tx.value) return { kind: 'tx', data: tx.value }
    if (block.status === 'fulfilled' && block.value) return { kind: 'block', data: block.value }
    return { kind: 'notfound' }
  }

  // 42-char hex = address
  if (/^0x[0-9a-fA-F]{40}$/.test(q)) return { kind: 'address', address: q }

  // Plain number = block height
  if (/^\d+$/.test(q)) {
    const block = await getBlockByNumber(parseInt(q, 10))
    if (block) return { kind: 'block', data: block }
    return { kind: 'notfound' }
  }

  return { kind: 'notfound' }
}
