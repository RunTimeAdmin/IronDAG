// ────────────────────────────────────────────────────────────────────────────
// IronDAG RPC response types
// All amounts are in attoIDAG (1 IDAG = 1e18 attoIDAG).
// Hex-encoded numbers arrive as "0x…" strings.
// ────────────────────────────────────────────────────────────────────────────

export type HexString = `0x${string}`
export type Address = `0x${string}`

// ── Block ────────────────────────────────────────────────────────────────────

export type StreamType = 'StreamA' | 'StreamB' | 'StreamC'

export interface RpcTransaction {
  hash: HexString
  from: Address
  to: Address | null
  value: HexString
  nonce: HexString
  gas: HexString
  gasPrice: HexString
  input: HexString
  blockHash: HexString | null
  blockNumber: HexString | null
  transactionIndex: HexString | null
  chainId?: HexString
  v?: HexString
  r?: HexString
  s?: HexString
}

export interface RpcBlock {
  hash: HexString
  number: HexString
  parentHash: HexString | HexString[]
  timestamp: HexString
  difficulty: HexString
  nonce: HexString
  miner: Address
  gasLimit: HexString
  gasUsed: HexString
  baseFeePerGas?: HexString
  transactions: RpcTransaction[] | HexString[]
  // IronDAG extensions
  stream?: StreamType
  streamType?: StreamType
}

// ── DAG stats ─────────────────────────────────────────────────────────────────

export interface DagStats {
  total_blocks?: number
  blue_blocks?: number
  red_blocks?: number
  dag_health?: number
  tps?: number
  avg_block_time?: number
  total_transactions?: number
}

// ── Stream blocks (idag_getBlocksByStream) ───────────────────────────────────

export interface StreamBlock {
  hash: HexString
  number: number
  stream: StreamType
  timestamp: number
  difficulty: number
  tx_count?: number
  transactions?: RpcTransaction[]
  miner?: Address
}

// ── Faucet ───────────────────────────────────────────────────────────────────

export interface FaucetResult {
  success: boolean
  tx_hash?: HexString
  message?: string
  balance?: HexString
}

// ── Address info ──────────────────────────────────────────────────────────────

export interface AddressInfo {
  address: Address
  balance: bigint
  nonce: number
  transactions: RpcTransaction[]
}

// ── Wallet ────────────────────────────────────────────────────────────────────

export interface WalletState {
  connected: boolean
  address: Address | null
  balance: bigint
  chainId: number | null
}
