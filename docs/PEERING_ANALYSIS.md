# Peering Configuration Analysis — Critical Issues

**Executive summary:** The P2P peering layer has critical gaps that prevent robust multi-node networking: no automatic bootstrap discovery (config exists but unused), peer exchange messages exist but are not used, advertise address is hardcoded to 127.0.0.1, and there is no peer persistence or quality management.

---

## 1. Bootstrap peer discovery

- **Location:** `config.rs` (bootstrap_peers), `node/mod.rs` (start).
- **Issue:** `NodeConfig::bootstrap_peers` is defined and defaulted to `vec![]` but **never read**. Peers are only connected via CLI `--peer` / positional args.
- **Impact:** No automatic connection to seed/bootstrap nodes; every node must be given explicit peer addresses.

---

## 2. Peer exchange not implemented

- **Location:** `network.rs` — `RequestPeers` and `Peers` message types and handlers.
- **Issue:** No code sends `RequestPeers`. The `Peers` handler only logs addresses and does not connect to them (“Could connect to these peers, but for now we just log”).
- **Impact:** Nodes cannot discover new peers from existing ones; network cannot grow or recover from partitions.

---

## 3. Advertise address

- **Location:** `network.rs` (handshake construction).
- **Issue:** When bind address is `0.0.0.0`, the handshake always advertises `127.0.0.1:port`. No config or CLI to set a public/advertise address.
- **Impact:** Public/VPS nodes advertise localhost; remote peers may try to connect to 127.0.0.1 and fail.

---

## 4. No peer persistence

- Peers are stored only in memory (`HashSet<SocketAddr>`). Restart loses all peers; reconnection is manual.

---

## 5. No peer quality / max peers

- No latency, uptime, or reputation tracking. `max_peers` exists in config but is not enforced when accepting or making connections.

---

## 6. Sync timing

- Single 1-second delay before starting sync after connect; no retry if the connection is not ready.

---

## Recommendations (priority)

1. **Use bootstrap peers** — In `Node::start()`, after starting the network, connect to every `config.bootstrap_peers` (with retries/logging).
2. **Configurable advertise address** — Add `advertise_addr: Option<String>` to config and network; use it in the handshake when set so VPS/public nodes can advertise their real address.
3. **Use peer exchange** — Send `RequestPeers` after connect (or periodically); in the `Peers` handler, connect to a bounded set of returned addresses (respecting max_peers).
4. **Enforce max_peers** — Reject or evict when at capacity.
5. **Peer persistence** — Optionally save/load peer list to disk.

---

## Implementation status (post–implementation guide)

The following fixes from the Peering Fixes Implementation Guide have been implemented:

| Item | Status | Notes |
|------|--------|--------|
| **Bootstrap peers** | Done | `Node::start()` connects to `config.bootstrap_peers`; 2s delay after bootstrap. CLI: `--bootstrap-peer <addr>` (repeatable). |
| **Advertise address** | Done | `NodeConfig::advertise_addr`, `NetworkManager::set_advertise_addr`, handshake uses it. CLI: `--advertise <addr>`. |
| **Peer exchange** | Done | `Peers` handler sends addresses to a channel; Node task receives and calls `connect_peer`. `request_peers_from(peer)` added. |
| **RequestPeers after Handshake** | Deferred | Would require passing `Arc<NetworkManager>` into `process_message` (type cycle). Periodic discovery (every 5 min) requests peers from all connected peers instead. |
| **Periodic peer discovery** | Done | Every 5 minutes, node requests peer list from each connected peer via `request_peers_from`. |
| **Max peers** | Done | `NetworkManager::max_peers` and `set_max_peers`; enforced in `connect_peer`. `node::NodeConfig::max_peers` (default 50). CLI: `--max-peers <n>`. |
| **Peer persistence** | Done | `load_peers(data_dir)` on startup; `save_peers` every 5 minutes to `data_dir/peers.json`. |
| **Peer quality / health / eviction** | Done | `PeerScore` (last_seen, success_count, failure_count); when at max_peers, evict lowest-scoring peer before accepting new (outgoing or incoming). See OPEN_ISSUES_PLAN Phase 4.3. |

*Full analysis and scenarios are in the original peering configuration document. This file summarizes issues and fixes.*
