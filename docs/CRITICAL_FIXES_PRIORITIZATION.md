# IronDAG Blockchain: Critical Fixes Prioritization

**Analysis Date:** 2024  
**Status:** Development Testnet  
**Total Issues Identified:** 23 critical issues across peering, CPU, and consensus/sharding.

---

## Status in this repository

*This section maps the prioritization list to the actual IronDAG codebase. Paths in the body below may refer to a generic structure; our paths are `irondag-blockchain/src/network.rs`, `mining.rs`, `pow.rs`, `node/mod.rs`, `consensus/`, `sharding/`.*

### P0 – Critical (5 items)

| # | Item | Status | Location / notes |
|---|------|--------|------------------|
| 1 | Fix advertise address | **Done** | `NodeConfig.advertise_addr`, `NetworkManager.set_advertise_addr`, handshake. CLI: `--advertise <addr>`. See [PEERING_ANALYSIS.md](PEERING_ANALYSIS.md). |
| 2 | Bootstrap peer connection | **Done** | `Node::start()` connects to `config.bootstrap_peers`; CLI: `--bootstrap-peer <addr>`. |
| 3 | Reduce Stream C frequency | **Done** | `STREAM_C_BLOCK_TIME` = 1s; Stream C off by default (`enable_stream_c: false`). CLI: `--enable-stream-c`. See [CPU_OPTIMIZATION.md](CPU_OPTIMIZATION.md). |
| 4 | Increase Stream B block time | **Done** | `pow::STREAM_B_TARGET_TIME` = 5s; mining uses it for sleep. |
| 5 | Reduce KHeavyHash memory | **Done** | `KHEAVY_MEMORY_SIZE` = 256KB, `KHEAVY_PASSES` = 2 in `pow.rs`. |

### P1 – High (7 items, doc items 6–12)

| # | Item | Status | Notes |
|---|------|--------|--------|
| 6 | Peer exchange | **Done** | `Peers` handler + channel; `request_peers_from()`; periodic discovery every 5 min. |
| 7 | Public IP discovery | **Not done** | STUN / auto-detect; manual `--advertise` only. |
| 8 | Periodic peer discovery | **Done** | Task every 5 min requests peers from all connected peers. |
| 9 | Cross-shard retry logic | **Not done** | See [BLOCKDAG_SHARDING_ANALYSIS.md](BLOCKDAG_SHARDING_ANALYSIS.md). |
| 10 | Persist async messaging | **Not done** | In-memory channels only. |
| 11 | Incremental DAG updates | **Not done** | Full O(n²) recalc on each block in `consensus/mod.rs`. |
| 12 | Replace Stream C with approval voting | **Not done** | Stream C is optional/slowed; no approval voting. |

### P2 – Medium (6 items)

| # | Item | Status |
|---|------|--------|
| 13 | LRU cache eviction (sharding) | Not done |
| 14 | Atomic cross-shard txs | Not done |
| 15 | Checkpointing and pruning (BlockDAG) | Not done |
| 16 | Peer quality management | Not done |
| 17 | Shard synchronization | Not done (placeholder) |
| 18 | Finality mechanism | Not done |

### P3 – Low (5 items)

| # | Item | Status |
|---|------|--------|
| 19 | Shard rebalancing | Not done |
| 20 | Peer persistence | **Done** | `peers.json` load/save. |
| 21 | Enforce max peers | **Done** | `max_peers` in config and network. |
| 22 | Network sync timing improvements | Partial (exponential backoff in handle_peer) |
| 23 | Cross-shard timeouts | Not done |

---

## Executive summary

Prioritized roadmap for 23 critical issues across:

- **Peering:** 10 issues (≈20–30 h to fix; many already done here).
- **CPU:** Multiple bottlenecks (≈5–10 h for critical fixes; done here).
- **BlockDAG/Sharding:** 10 issues (≈90–130 h to fix).

Issues are ranked by **business impact × effort** to maximize ROI.

### Quick stats

- **P0 – Critical:** 5 issues, 5–10 h → production blocker. *(All P0 items are implemented in this repo.)*
- **P1 – High:** 8 issues, 15–25 h → major performance/resilience. *(Several done: peer exchange, periodic discovery; rest not done.)*
- **P2 – Medium:** 6 issues, 30–50 h → scalability.
- **P3 – Low:** 4 issues, 60–80 h → long-term viability.

---

## P0 – CRITICAL (5–10 hours) — *Implemented in this repo*

1. **Fix advertise address** — Use config address if provided; else listen address. *Done: `advertise_addr`, `set_advertise_addr`, handshake, `--advertise`.*
2. **Bootstrap peer connection** — Connect to `bootstrap_peers` after starting listener. *Done: in `Node::start()`, plus `--bootstrap-peer`.*
3. **Reduce Stream C frequency** — 100ms → 1s; optional by default. *Done: `STREAM_C_BLOCK_TIME` = 1s, `enable_stream_c` default false.*
4. **Increase Stream B block time** — 1s → 5s. *Done: `STREAM_B_TARGET_TIME` = 5.*
5. **Reduce KHeavyHash memory** — 1MB → 256KB (and 3 → 2 passes). *Done in `pow.rs`.*

**P0 combined:** CPU and network critical path addressed; testnet can bootstrap and run with lower CPU.

---

## P1 – HIGH (15–25 hours)

6. **Peer exchange** — Request/send peer lists; connect to new peers. *Done: channel + Peers handler + `request_peers_from` + periodic discovery.*  
7. **Public IP discovery** — STUN or fallback so nodes behind NAT can advertise. *Not done.*  
8. **Periodic peer discovery** — Loop to maintain peers and request peer lists. *Done: 5 min interval.*  
9. **Cross-shard retry logic** — Retry queue, backoff, optional persistence. *Not done.*  
10. **Persist async messaging** — WAL + replay for cross-shard messages. *Not done.*  
11. **Incremental DAG updates** — Only recalc affected subtrees; avoid full O(n²). *Not done.*  
12. **Replace Stream C with approval voting** — Optional; not implemented.

---

## P2 – MEDIUM (30–50 hours)

13. LRU cache eviction (sharding)  
14. Atomic cross-shard transactions (e.g. two-phase commit)  
15. Checkpointing and pruning (BlockDAG)  
16. Peer quality management  
17. Shard synchronization (real implementation)  
18. Finality mechanism  

---

## P3 – LOW (60–80 hours)

19. Shard rebalancing  
20. Peer persistence — *Done: `peers.json`.*  
21. Enforce max peers — *Done.*  
22. Network sync timing — *Partial (backoff).*  
23. Cross-shard timeouts  

---

## Implementation roadmap (reference)

- **Phase 0 (P0):** 5–10 h — *Completed in this repo.*  
- **Phase 1 (P1):** 15–25 h — Peer automation done; cross-shard + DAG + optional approval voting remain.  
- **Phase 2 (P2):** 30–50 h — Scalability and consistency.  
- **Phase 3 (P3):** 60–80 h — Production hardening.  

**Suggested next focus:** P1 items 7 (public IP), 9 (cross-shard retry), 10 (async persistence), 11 (incremental DAG). See [BLOCKDAG_SHARDING_ANALYSIS.md](BLOCKDAG_SHARDING_ANALYSIS.md) for details.

---

*Document version: 1.0. Paths and status above reflect this repository; body structure follows the original prioritization document.*
