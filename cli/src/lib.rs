//! `scribe-cli` - a thin, standalone client for Scribe's dictation-state
//! interface.
//!
//! This library speaks only the frozen HTTP+SSE wire contract (v1): it reads
//! `~/.scribe/control.json` for a `baseUrl` + `readToken`, GETs `/v1/status`
//! for a point-in-time snapshot, and streams `/v1/events` (SSE) for live
//! changes. It is decoupled from Scribe's Tauri app and makes no assumptions
//! about who is consuming it.
//!
//! Hard invariant it upholds (contract section 7): stale-or-dead ALWAYS
//! resolves to not-dictating. A missing/unparseable control file, a dead pid,
//! or a refused/closed connection all surface as a not-dictating snapshot.

pub mod client;
pub mod config;
pub mod discovery;
pub mod liveness;
pub mod output;
pub mod snapshot;

pub use client::{fetch_status, watch, ClientError, FetchedSnapshot, WatchItem, WatchOptions};
pub use config::{resolve, Offline, Resolution, ResolveOptions, Target};
pub use discovery::Channel;
pub use snapshot::Snapshot;
