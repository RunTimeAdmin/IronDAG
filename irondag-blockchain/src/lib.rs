//! IronDAG Blockchain
//!
//! High-performance sharded blockchain with BraidCore mining architecture
//! and GhostDAG consensus.
//!
//! Copyright (c) 2024-2025 IronDAG Contributors
//! Licensed under the BUSL-1.1 License (see LICENSE file)

// Clippy lint configuration
#![allow(clippy::needless_borrows_for_generic_args)]
#![allow(clippy::type_complexity)]
#![allow(clippy::unnecessary_cast)]
#![allow(clippy::unwrap_or_default)]
#![allow(clippy::collapsible_if)]
#![allow(clippy::map_entry)]
#![allow(clippy::bind_instead_of_map)]
#![allow(clippy::needless_borrow)]
#![allow(clippy::manual_range_contains)]
#![allow(clippy::manual_is_multiple_of)]
#![allow(clippy::new_without_default)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::get_first)]
#![allow(clippy::redundant_closure)]
#![allow(clippy::manual_strip)]
#![allow(clippy::len_zero)]
#![allow(clippy::redundant_pattern_matching)]
#![allow(clippy::manual_split_once)]
#![allow(clippy::collapsible_else_if)]
#![allow(clippy::bool_assert_comparison)]
#![allow(clippy::implicit_saturating_sub)]
#![allow(clippy::needless_return)]
#![allow(clippy::for_kv_map)]
#![allow(clippy::should_implement_trait)]
#![allow(clippy::single_match)]
#![allow(clippy::large_enum_variant)]
#![allow(clippy::useless_vec)]
#![allow(clippy::unused_async)]
#![allow(clippy::bool_comparison)]

pub mod account_abstraction;
pub mod blockchain;
pub mod config;
pub mod consensus;
pub mod error;
pub mod evm;
pub mod gas_sponsorship;
pub mod governance;
pub mod light_client;
pub mod metrics;
pub mod mining;
pub mod network;
pub mod node;
pub mod noise;
pub mod oracles;
pub mod pow;
pub mod pqc;
#[cfg(feature = "privacy")]
pub mod privacy;
pub mod privacy_pool;
pub mod quic_transport;
pub mod recurring;
pub mod reputation;
pub mod rpc;
pub mod security;
pub mod sharding;
pub mod stop_loss;
pub mod storage;
pub mod types;
pub mod verkle;
pub mod zk; // 3 arkworks 0.4 API errors - optional module, doesn't block core functionality
