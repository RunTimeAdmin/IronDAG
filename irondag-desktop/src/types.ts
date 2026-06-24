// Type definitions for IronDAG Desktop

export type NodeStatus = {
  height: number;
  tx_count: number;
  peer_count: number;
  is_mining: boolean;
};

export type MiningStatus = {
  is_mining: boolean;
  pending_txs: number;
  streams: {
    streamA: { block_time_ms: number; max_txs: number; reward: string };
    streamB: { block_time_ms: number; max_txs: number; reward: string };
    streamC: { block_time_ms: number; max_txs: number; reward: string };
  };
};

export type Block = {
  number: string;
  hash: string;
  timestamp: string;
  transactions: any[];
  stream_type?: string;
  streamType?: string;
  parentHashes?: string[];
  parentHash?: string;
};

export type DagStats = {
  total_blocks: number;
  blue_blocks: number;
  red_blocks: number;
  total_transactions: number;
  avg_txs_per_block: number;
};

export type ShardStats = {
  shard_count: number;
  shards: Array<{
    shard_id: number;
    block_count: number;
    transaction_pool_size: number;
    cross_shard_outgoing: number;
    cross_shard_incoming: number;
  }>;
};

export type NodeProcessInfo = {
  id: string;
  pid: number;
  p2p_port: number;
  rpc_port: number;
  http_port?: number | null;
  data_dir?: string | null;
  enable_mining: boolean;
  peers: string[];
  log_streaming: boolean;
  exit_code?: number | null;
};

export type TestDefinition = {
  name: string;
  label: string;
  description: string;
};

export type TestResult = {
  name: string;
  exit_code: number;
  stdout: string;
  stderr: string;
  duration_ms: number;
};

export type Contact = {
  name: string;
  address: string;
  notes?: string;
};

export type Account = {
  name: string;
  address: string;
};

export type WalletType = "basic" | "multisig" | "social" | "spending" | "combined";

export type SmartWallet = {
  address: string;
  wallet_type: string;
  owner: string;
  nonce?: number;
};

export type Reputation = {
  score: number;
  level: "High" | "Medium" | "Low";
};

export type ReputationFactors = {
  successful_txs: number;
  failed_txs: number;
  blocks_mined: number;
  account_age_days: number;
  total_value_transacted: number;
  unique_contacts: number;
};

export type Transaction = {
  hash: string;
  from: string;
  to: string;
  value: string;
  block_number: string;
  timestamp: string;
  direction?: "incoming" | "outgoing";
};

export type PrivacyStats = {
  total_private_txs: number;
  nullifier_count: number;
  enabled: boolean;
};

export type PriceFeed = {
  feed_id?: string;
  symbol?: string;
  price?: string;
};

export type RecurringTransaction = {
  recurring_tx_id: string;
  from: string;
  to: string;
  value: string;
  status: string;
  execution_count: number;
};

export type StopLossOrder = {
  stop_loss_id: string;
  token_symbol?: string;
  amount: string;
  trigger_price: string;
  order_type?: "sell" | "buy";
  status?: string;
  owner?: string;
};

export type MiningDashboard = {
  total_earnings_recent: string;
  recent_sample_size: number;
  total_blocks: number;
  streams: {
    stream_a: {
      blocks_mined: number;
      hashrate_estimate_blocks_per_hour: number;
      earnings: string;
    };
    stream_b: {
      blocks_mined: number;
      hashrate_estimate_blocks_per_hour: number;
      earnings: string;
    };
    stream_c: {
      blocks_mined: number;
      hashrate_estimate_blocks_per_hour: number;
      fees_collected: string;
    };
  };
};

export type ParallelEVMStats = {
  enabled: boolean;
  maxParallel?: number;
  stats?: {
    avgSpeedup?: number;
    parallelRate?: number;
  };
};

export type ConfirmDialogState = {
  title: string;
  message: string;
  onConfirm: () => void;
} | null;

export type TabType = 
  | "dashboard" 
  | "wallet" 
  | "send" 
  | "history" 
  | "explorer" 
  | "metrics" 
  | "account-abstraction" 
  | "privacy" 
  | "oracles" 
  | "recurring" 
  | "stop-loss"
  | "settings";

export type UpdateInfo = {
  version: string;
  body: string;
} | null;
