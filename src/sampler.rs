//! Multi-threaded metrics sampler.
//!
//! Spawns one thread per metric category. All threads write into a shared
//! `Arc<Mutex<Metrics>>`. The `snapshot()` method clones the current state
//! for the TUI / JSON consumer to read without blocking the writers.

use crate::cpu;
use crate::types::*;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// SSD power model constants (linear interpolation between idle and max).
const SSD_IDLE_W: f32 = 0.03;
const SSD_MAX_ACTIVE_W: f32 = 2.5;

pub struct Sampler {
    shared: Arc<Mutex<Metrics>>,
    // Join handles are held but never joined — threads run until the process exits.
    _handles: Vec<std::thread::JoinHandle<()>>,
}

impl Sampler {
    /// Create a new sampler with the given polling interval (ms, clamped to ≥ 100).
    ///
    /// CPU model, core count, and TDP are detected in parallel via a scoped thread
    /// to avoid blocking the main thread at startup.
    pub fn new(interval_ms: u64) -> Self {
        let (cpu_model, num_cores, tdp_w) = std::thread::scope(|s| {
            let h1 = s.spawn(|| cpu::read_cpu_model());
            let h2 = s.spawn(|| cpu::read_num_cores());
            let h3 = s.spawn(|| cpu::detect_tdp_w());
            (
                h1.join().unwrap_or_default(),
                h2.join().unwrap_or(0),
                h3.join().unwrap_or(45.0),
            )
        });

        let shared = Arc::new(Mutex::new(Metrics {
            cpu: CpuInfo {
                model: cpu_model.clone(),
                num_cores,
                ..Default::default()
            },
            ..Default::default()
        }));
        let mut handles = Vec::new();
        let dt = Duration::from_millis(interval_ms.max(100));

        // ── Thread 1: RAPL power (CPU domains) ────────────────────────────
        {
            let m = shared.clone();
            handles.push(std::thread::spawn(move || {
                let mut prev = cpu::discover_rapl_domains();
                let mut prev_time = Instant::now();
                if prev.is_empty() {
                    return; // No RAPL support — other threads will use fallback
                }
                loop {
                    std::thread::sleep(dt);
                    let cur = cpu::discover_rapl_domains();
                    let now = Instant::now();
                    let dt_s = now.duration_since(prev_time).as_secs_f64();
                    let cpu_power = cpu::compute_rapl_power(&prev, &cur, dt_s);
                    if let Ok(mut mg) = m.lock() {
                        mg.cpu.power = cpu_power;
                    }
                    prev = cur;
                    prev_time = now;
                }
            }));
        }

        // ── Thread 2: CPU utilization + frequency ─────────────────────────
        {
            let m = shared.clone();
            handles.push(std::thread::spawn(move || {
                let mut prev_ticks = cpu::read_cpu_ticks();
                loop {
                    std::thread::sleep(dt);
                    let cur_ticks = cpu::read_cpu_ticks();
                    let cpu_usage = cpu::compute_cpu_usage(&prev_ticks, &cur_ticks);
                    let cpu_freqs = cpu::read_cpu_freqs();
                    if let Ok(mut mg) = m.lock() {
                        mg.cpu.usage_pct = cpu_usage;
                        mg.cpu.freq_mhz = cpu_freqs;
                    }
                    prev_ticks = cur_ticks;
                }
            }));
        }

        // ── Thread 3: Battery + AC adapter ────────────────────────────────
        {
            let m = shared.clone();
            handles.push(std::thread::spawn(move || {
                // 5-minute SMA used to smooth the time-remaining estimate.
                let mut power_sma = crate::sma::TimeSma::new(300.0);
                loop {
                    let mut b = crate::battery::read_battery();
                    let adapter = crate::battery::read_adapter();
                    b.external_connected = adapter.online;

                    // If sysfs doesn't report drain directly, fall back to
                    // the system power estimate from other threads.
                    if b.present && b.drain_w == 0.0 {
                        if let Ok(mg) = m.lock() {
                            if !b.external_connected && mg.sys_power_w > 0.0 {
                                b.drain_w = mg.sys_power_w as f64;
                            }
                        }
                    }

                    if b.present {
                        power_sma.push(b.drain_w.abs() as f32);
                    }

                    // Estimate time remaining from average power when the
                    // kernel doesn't provide it.
                    if b.present && b.time_remaining_min <= 0 && b.energy_full_wh > 0.0 {
                        let avg_power = power_sma.get() as f64;
                        if avg_power > 0.5 {
                            let remaining_wh = b.energy_full_wh * b.percent / 100.0;
                            b.time_remaining_min = if b.external_connected {
                                let full_wh = b.energy_full_wh - remaining_wh;
                                (full_wh / avg_power * 60.0) as i64
                            } else {
                                (remaining_wh / avg_power * 60.0) as i64
                            };
                        }
                    }

                    if let Ok(mut mg) = m.lock() {
                        mg.battery = b;
                        mg.adapter = adapter;
                    }
                    std::thread::sleep(dt);
                }
            }));
        }

        // ── Thread 4: GPU ─────────────────────────────────────────────────
        // Polled at 1s since GPU state changes slowly and nvidia-smi is expensive.
        {
            let m = shared.clone();
            handles.push(std::thread::spawn(move || loop {
                let gpu = crate::gpu::read_gpu_info();
                if let Ok(mut mg) = m.lock() {
                    mg.gpu = gpu;
                }
                std::thread::sleep(Duration::from_millis(1000));
            }));
        }

        // ── Thread 5: Temperatures + Fans ─────────────────────────────────
        {
            let m = shared.clone();
            handles.push(std::thread::spawn(move || loop {
                let temps = crate::thermal::read_temperatures();
                let fans = crate::thermal::read_fans();
                // Core temps need the core count to size the output vector.
                let num_cores = if let Ok(mg) = m.lock() {
                    mg.cpu.num_cores
                } else {
                    0
                };
                let core_temps = crate::thermal::read_core_temps(num_cores);
                if let Ok(mut mg) = m.lock() {
                    mg.temperatures = temps;
                    mg.fans = fans;
                    mg.cpu.core_temps = core_temps;
                }
                std::thread::sleep(dt);
            }));
        }

        // ── Thread 6: Network ─────────────────────────────────────────────
        {
            let m = shared.clone();
            handles.push(std::thread::spawn(move || {
                let mut prev = crate::network::read_net_counters();
                let mut prev_time = Instant::now();
                loop {
                    std::thread::sleep(dt);
                    let cur = crate::network::read_net_counters();
                    let now = Instant::now();
                    let dt_s = now.duration_since(prev_time).as_secs_f64();
                    let rates = crate::network::compute_net_rates(&prev, &cur, dt_s);
                    if let Ok(mut mg) = m.lock() {
                        mg.network = rates;
                    }
                    prev = cur;
                    prev_time = now;
                }
            }));
        }

        // ── Thread 7: Disk I/O + SSD power ────────────────────────────────
        {
            let m = shared.clone();
            handles.push(std::thread::spawn(move || {
                let mut prev = crate::disk::read_disk_counters();
                let mut prev_time = Instant::now();
                loop {
                    std::thread::sleep(dt);
                    let cur = crate::disk::read_disk_counters();
                    let now = Instant::now();
                    let dt_s = now.duration_since(prev_time).as_secs_f64();
                    let disk = crate::disk::compute_disk_rates(prev, cur, dt_s);
                    // SSD power: linear model based on I/O utilization
                    let total_bps = disk.read_bytes_per_sec + disk.write_bytes_per_sec;
                    let max_bps = 3_000.0 * 1024.0 * 1024.0; // 3 GB/s ≈ SATA/NVMe ceiling
                    let util = (total_bps / max_bps).clamp(0.0, 1.0) as f32;
                    let ssd_power = SSD_IDLE_W + util * (SSD_MAX_ACTIVE_W - SSD_IDLE_W);
                    if let Ok(mut mg) = m.lock() {
                        mg.disk = disk;
                        mg.disk_ssd_power_w = ssd_power;
                    }
                    prev = cur;
                    prev_time = now;
                }
            }));
        }

        // ── Thread 8: Memory ──────────────────────────────────────────────
        {
            let m = shared.clone();
            handles.push(std::thread::spawn(move || loop {
                let (total_gb, used_gb) = read_mem_info();
                if let Ok(mut mg) = m.lock() {
                    mg.mem_total_gb = total_gb;
                    mg.mem_used_gb = used_gb;
                }
                std::thread::sleep(dt);
            }));
        }

        // ── Thread 9: Per-process stats ───────────────────────────────────
        {
            let m = shared.clone();
            handles.push(std::thread::spawn(move || {
                // Maps PID → (utime, stime, read_bytes, write_bytes, timestamp)
                let mut prev_ticks: HashMap<i32, (u64, u64, u64, u64, Instant)> = HashMap::new();
                loop {
                    let (procs, new_ticks) =
                        crate::process::read_top_processes(&prev_ticks, num_cores, 50);
                    if let Ok(mut mg) = m.lock() {
                        mg.top_processes = procs;
                    }
                    prev_ticks = new_ticks;
                    std::thread::sleep(dt);
                }
            }));
        }

        // ── Thread 10: System power aggregation ───────────────────────────
        {
            let m = shared.clone();
            handles.push(std::thread::spawn(move || {
                loop {
                    if let Ok(mut mg) = m.lock() {
                        let mut total = mg.cpu.power.total_w;
                        total += mg.gpu.power_w;
                        total += mg.fans.iter().map(|f| f.estimated_power_w).sum::<f32>();

                        // When RAPL is unavailable, estimate CPU power from
                        // utilization and the auto-detected TDP.
                        if mg.cpu.power.total_w == 0.0 && !mg.cpu.usage_pct.is_empty() {
                            let avg_usage: f32 = mg.cpu.usage_pct.iter().sum::<f32>()
                                / mg.cpu.usage_pct.len() as f32;
                            let idle_w = tdp_w * 0.1; // ~10% of TDP at idle
                            let estimated = idle_w + (avg_usage / 100.0) * (tdp_w - idle_w);
                            total += estimated;
                            mg.cpu.power.total_w = estimated;
                            mg.cpu.power.available = true;
                        }

                        // When discharging on battery, use battery drain as the
                        // authoritative system power (more accurate than summing
                        // components).
                        mg.sys_power_w = if mg.battery.present && !mg.battery.external_connected {
                            mg.battery.drain_w.abs() as f32
                        } else {
                            total
                        };
                    }
                    std::thread::sleep(dt);
                }
            }));
        }

        Self {
            shared,
            _handles: handles,
        }
    }

    /// Take a consistent snapshot of all metrics by cloning under the lock.
    /// Uses `into_inner` on poison to keep working even if a writer panicked.
    pub fn snapshot(&self) -> Metrics {
        self.shared
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

/// Read total and used memory (GB) from `/proc/meminfo`.
/// Used = MemTotal − MemAvailable (includes buffers/caches in "available").
fn read_mem_info() -> (f32, f32) {
    let Ok(content) = std::fs::read_to_string("/proc/meminfo") else {
        return (0.0, 0.0);
    };
    let mut total_kb: f64 = 0.0;
    let mut available_kb: f64 = 0.0;
    for line in content.lines() {
        if let Some(val) = line.strip_prefix("MemTotal:") {
            total_kb = val
                .trim()
                .split_whitespace()
                .next()
                .unwrap_or("0")
                .parse()
                .unwrap_or(0.0);
        } else if let Some(val) = line.strip_prefix("MemAvailable:") {
            available_kb = val
                .trim()
                .split_whitespace()
                .next()
                .unwrap_or("0")
                .parse()
                .unwrap_or(0.0);
        }
    }
    let total_gb = (total_kb / 1024.0 / 1024.0) as f32;
    let used_gb = ((total_kb - available_kb) / 1024.0 / 1024.0) as f32;
    (total_gb, used_gb)
}
