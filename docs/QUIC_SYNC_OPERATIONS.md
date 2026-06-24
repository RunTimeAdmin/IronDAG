# QUIC P2P and block sync — operations

**Last Updated: 2026-04-10 (Deployed on Testnet)**

**Purpose:** Explain how peering relates to sync, QUIC settings, and what to check when `net_peerCount > 0` but the chain does not catch up.

## Transport

- **P2P uses QUIC over UDP** on the port passed to `--port` (not TCP).
- **Gossip** uses bidirectional streams with leading byte `0x01` (`STREAM_TYPE_GOSSIP`).
- **IBD / height / block batch sync** uses separate bidirectional streams with leading byte `0x02` (`STREAM_TYPE_SYNC`).

Files of interest: `src/quic_transport.rs`, `src/network.rs` (`handle_connection_streams`, `connect_peer`), `src/network/sync.rs` (`SyncClient::full_sync_quic`).

## QUIC configuration (CLI)

| Flag | Default | Role |
|------|---------|------|
| `--port` | 8080 | **UDP** listen/connect address for QUIC |
| `--quic-idle-timeout <SECS>` | 30 | Max idle time before Quinn closes the connection (allowed range in CLI: 5–300) |

Internal tuning (see `quic_transport::create_endpoint`):

- **Idle timeout:** `quic_idle_timeout_secs` (from `--quic-idle-timeout`).
- **Keep-alive:** 10s interval to reduce idle disconnects during long operations.

If sync stalls on large chains, try `--quic-idle-timeout 120` on both ends.

## Staying in sync after IBD

- Nodes with `--peer` run a **background catch-up loop**: `full_sync_quic` runs, then again every **15 seconds** while the QUIC session to that peer exists.
- During a single `full_sync_quic` run, the client **re-queries peer height before each download batch**, so a mining peer’s tip does not outrun one IBD pass.
- **GetBlocks** responses are built in **ascending `block_number` order** (not sled append order), with **transitive parents** included so BraidCore / BlockDAG edges are not missing when fast streams interleave in storage.
- The sync client advances the next `from_block` from **max known block number + 1**, not “batch size”, so failed inserts do not skip ranges.

## Firewall and hosting

- Open **UDP** (and optionally TCP only if you run something else on TCP — P2P here is **UDP**).
- Source and destination must match the same IP family (IPv4 vs IPv6) and port you pass in `--peer`.

## When peering works but sync does not

### 1. Sync only runs from the node that called `connect_peer`

`Node::connect_peer` (wired from `--peer`) schedules repeated `SyncClient::full_sync_quic` (after handshake, then every 15s).

- **Follower** nodes must use `--peer <miner-public-ip>:<miner-udp-port>`.
- The **miner** accepting inbound peers does not pull followers; followers pull from configured peers.

### 2. "Already synced" (0 blocks added)

`full_sync_quic` compares heights using `blockchain.get_blocks().len()` as the peer height (from signed `GetHeight`). If **local length ≥ peer length**, sync exits with 0 new blocks. That can happen if:

- Follower already has the same blocks (e.g. same genesis + local mining).
- Different **data dirs** / wiped miner vs stale follower — verify both sides point at the intended `--data-dir`.

### 3. `No QUIC connection found for peer` (logs)

The background sync task uses `get_peer_connection(addr)` for the **exact** `SocketAddr` passed to `connect_peer`. If setup is wrong (e.g. connection dropped before 500ms, or address mismatch), sync aborts. Check logs on the follower for this warning.

### 4. Signature / time skew

Signed sync responses use a **±5 minute** timestamp window (`SignedSyncResponse::verify`). Severe clock skew between VPS nodes can cause verify failures and apparent “no sync”. Keep NTP enabled on all nodes.

### 5. Inbound sync handler

The mining/listening node answers sync on **incoming** QUIC streams: `handle_sync_stream` reads `MSHWSYNC` + version + request. If you see `Invalid magic` or `Sync stream handling failed` in logs on the **peer that should serve blocks**, investigate version/bincode mismatches or corrupted stream framing.

## What to look for in logs

**Follower (sync client):**

- `🔄 [SYNC] Starting QUIC full sync with ...`
- `✅ [SYNC] Verified height ...` or `❌ [SYNC DEBUG] get_peer_height_quic failed: ...`
- `✅ [SYNC] Already synced (local: X, peer: Y)` — explains zero progress when Y ≤ X

**Miner / serving node:**

- `📤 [SYNC] Sending height N to ...`
- `📥 [SYNC] ... requesting blocks from ...`

## Next steps (recommended)

1. Confirm **UDP** is open for `--port` on the miner and that `--peer` uses the **same port** the miner listens on.
2. On a follower, run with `tracing`/`RUST_LOG` if enabled, or watch stdout for the `[SYNC]` lines above.
3. If idle drops occur mid-IBD, raise `--quic-idle-timeout` on both peers.
4. If heights look wrong, compare `eth_blockNumber` / local block count on both nodes and verify `--data-dir`.

---

## Troubleshooting: Sync Stall Recovery

### Symptom: "blocks have unresolvable parents" repeating

**Root Cause:** All blocks in a downloaded batch reference parents the node doesn't have. Fixed in commits 235d484/da30fc6/cb79df4.

**Current Behavior:** Orphaned blocks are retained across batches and retried. Sync advances past all-orphaned batches. No operator intervention needed.

**If running old binary:** Update to latest, wipe data, restart:
```bash
systemctl stop irondag-node
cd /opt/irondag/irondag-blockchain
git pull origin feature/alpha-hardening
cargo build --release --features kyber
rm -rf data
systemctl start irondag-node
```

## Troubleshooting: Fork Recovery

### Symptom: Node rejects all peer blocks after restart

**Root Cause:** Stale DAG edges in sled storage from previous chain. Fixed in commit 16105c7.

**Current Behavior:** When peer has 50+ more blocks than local, all sled data is automatically cleared and resync starts from genesis. No operator intervention needed.

**Manual recovery:**
```bash
systemctl stop irondag-node
rm -rf /opt/irondag/irondag-blockchain/data
systemctl start irondag-node
```

## Troubleshooting: Mining During Sync

### Symptom: Node mines blocks that peers reject

**Root Cause:** Mining produces blocks on stale DAG tips while syncing a larger chain. Fixed in commit cb1b2b4.

**Current Behavior:** Mining is automatically paused during IBD and resumed after sync completes. Look for these log lines:
```
⏸️ [MINING] Paused during initial block download
▶️ [MINING] Resumed after sync complete
```

---

## Testnet Status (Apr 2026)

- QUIC P2P active with 3 connected peers
- Full IBD sync working (Frankfurt syncs full chain from .31)
- Gossip block propagation working
- All sync stall fixes deployed and verified
- Block height 150+
