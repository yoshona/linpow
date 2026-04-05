//! **linpow** — Linux real-time power consumption monitor.
//!
//! Architecture overview:
//!
//! ```text
//! main.rs
//!   ├─ Sampler (multi-threaded metrics collection)
//!   │     spawns dedicated threads for:
//!   │     RAPL power · CPU util/freq · Battery · GPU · Thermal/Fans
//!   │     Network · Disk I/O · Memory · Per-process · System power
//!   │     → writes into Arc<Mutex<Metrics>>
//!   │
//!   └─ App (TUI rendering)  or  JSON stream
//!         reads Metrics via channel or snapshot
//! ```

pub mod app;
pub mod battery;
pub mod cpu;
pub mod disk;
pub mod gpu;
pub mod network;
pub mod process;
pub mod sampler;
pub mod sma;
pub mod thermal;
pub mod types;
