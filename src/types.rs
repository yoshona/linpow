//! Shared data types used across all modules.

use clap::Parser;
use serde::Serialize;

// ── CLI ──────────────────────────────────────────────────────────────────────

#[derive(Parser, Debug)]
#[command(name = env!("CARGO_PKG_NAME"), version, about = "Linux Power Monitor TUI")]
pub struct CliArgs {
    /// Sampling interval in milliseconds
    #[arg(long, default_value_t = 250)]
    pub interval: u64,

    /// Output JSON to stdout instead of TUI
    #[arg(long)]
    pub json: bool,
}

// ── Top-level metrics snapshot ──────────────────────────────────────────────

/// Complete system metrics snapshot, produced by the sampler and consumed by the TUI / JSON.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Metrics {
    pub cpu: CpuInfo,
    pub gpu: GpuInfo,
    pub battery: BatteryInfo,
    pub adapter: AdapterInfo,
    pub fans: Vec<FanInfo>,
    pub temperatures: Vec<TempSensor>,
    pub network: Vec<InterfaceRate>,
    pub disk: DiskInfo,
    pub mem_total_gb: f32,
    pub mem_used_gb: f32,
    pub top_processes: Vec<ProcessInfo>,
    /// Aggregated system power (sum of all components, or battery drain when discharging).
    pub sys_power_w: f32,
    pub disk_ssd_power_w: f32,
}

// ── CPU ──────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize)]
pub struct CpuInfo {
    pub power: CpuPower,
    pub usage_pct: Vec<f32>,
    pub freq_mhz: Vec<u32>,
    pub core_temps: Vec<f32>,
    pub model: String,
    pub num_cores: usize,
}

/// A single RAPL energy domain (e.g. "package-0.0", "core", "dram").
#[derive(Debug, Clone, Default, Serialize)]
pub struct RaplDomain {
    pub name: String,
    pub power_w: f32,
    pub energy_uj: u64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct CpuPower {
    pub available: bool,
    /// Power drawn by the CPU package (includes cores, uncore, DRAM on some CPUs).
    pub package_w: f32,
    /// All discovered RAPL domains with their per-domain power.
    pub domains: Vec<RaplDomain>,
    /// Total CPU power — set to package_w (package already includes sub-domains).
    pub total_w: f32,
}

// ── GPU ──────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize)]
pub struct GpuInfo {
    pub available: bool,
    pub vendor: GpuVendor,
    pub name: String,
    pub power_w: f32,
    pub utilization_pct: u32,
    pub memory_used_mb: u64,
    pub memory_total_mb: u64,
    pub temperature_c: f32,
    pub core_clock_mhz: u32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub enum GpuVendor {
    #[default]
    None,
    Nvidia,
    Amd,
    Intel,
}

// ── Battery ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize)]
pub struct BatteryInfo {
    pub present: bool,
    pub status: String,
    pub percent: f64,
    pub voltage_mv: f64,
    pub current_ma: f64,
    pub power_w: f64,
    pub energy_now_wh: f64,
    pub energy_full_wh: f64,
    /// Positive = discharging, negative = charging.
    pub drain_w: f64,
    pub time_remaining_min: i64,
    pub external_connected: bool,
    pub cycle_count: i64,
    pub health_pct: f64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct AdapterInfo {
    pub online: bool,
}

// ── Thermal / Fans ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize)]
pub struct FanInfo {
    pub name: String,
    pub rpm: f32,
    pub min_rpm: f32,
    pub max_rpm: f32,
    /// Estimated power draw using a cubic fan model (W).
    pub estimated_power_w: f32,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct TempSensor {
    pub label: String,
    /// Category for grouping in the TUI: CPU, GPU, Memory, SSD, Battery, Board, Other.
    pub category: String,
    pub value_celsius: f32,
}

// ── Network ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize)]
pub struct InterfaceRate {
    pub name: String,
    pub bytes_in_per_sec: f64,
    pub bytes_out_per_sec: f64,
    pub is_wifi: bool,
    pub link_speed_mbps: u32,
    pub wifi_ssid: String,
    pub wifi_rssi_dbm: i32,
}

// ── Disk ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize)]
pub struct DiskInfo {
    pub read_bytes_per_sec: f64,
    pub write_bytes_per_sec: f64,
    pub disk_name: String,
}

// ── Processes ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize)]
pub struct ProcessInfo {
    pub pid: i32,
    pub name: String,
    /// CPU % in htop style (100% = one core fully utilized).
    pub cpu_pct: f32,
    pub rss_mb: f64,
    pub disk_read_bytes_per_sec: f64,
    pub disk_write_bytes_per_sec: f64,
    pub disk_read_bytes: u64,
    pub disk_write_bytes: u64,
    pub alive: bool,
}
