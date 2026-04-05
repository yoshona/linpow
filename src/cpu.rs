//! CPU metrics: RAPL power domains, per-core utilization, frequency, model, and TDP detection.

use crate::types::{CpuPower, RaplDomain};
use std::fs;
use std::path::Path;

// ── RAPL power ───────────────────────────────────────────────────────────────

/// Discover all RAPL energy domains under `/sys/class/powercap/`.
///
/// On Intel the hierarchy is:
/// ```text
/// intel-rapl:0          → name = "package-0"
///   intel-rapl:0:0      → name = "core"
///   intel-rapl:0:1      → name = "dram"
/// ```
/// On AMD it is analogous but uses the `amd_rapl` prefix.
///
/// The top-level (package) domain name is suffixed with `.0` to avoid
/// colliding with sub-domains that may share a bare name.
pub fn discover_rapl_domains() -> Vec<RaplDomain> {
    let mut domains = Vec::new();
    let base = Path::new("/sys/class/powercap");
    if !base.exists() {
        return domains;
    }

    let Ok(entries) = fs::read_dir(base) else {
        return domains;
    };

    let mut rapl_entries: Vec<_> = entries
        .filter_map(|e| e.ok())
        .filter(|e| {
            let binding = e.file_name();
            let name = binding.to_string_lossy();
            name.starts_with("intel-rapl") || name.starts_with("amd_rapl")
        })
        .collect();
    rapl_entries.sort_by_key(|e| e.file_name());

    for entry in &rapl_entries {
        let path = entry.path();

        // Top-level domain (package)
        let name = read_sysfs_string(&path.join("name"));
        let energy_uj = read_sysfs_u64(&path.join("energy_uj")).unwrap_or(0);
        if !name.is_empty() {
            domains.push(RaplDomain {
                name: format!("{}.0", name),
                power_w: 0.0,
                energy_uj,
            });
        }

        // Determine correct prefix for sub-domains
        let entry_name = entry.file_name().to_string_lossy().to_string();
        let prefix = if entry_name.starts_with("amd_rapl") {
            "amd_rapl"
        } else {
            "intel-rapl"
        };
        let parent_idx = entry_name
            .split(':')
            .nth(1)
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(0);

        // Iterate sub-domains (e.g. intel-rapl:0:0, amd_rapl:0:0)
        let mut sub_idx = 0;
        loop {
            let sub_path = path.join(format!("{}:{}:{}", prefix, parent_idx, sub_idx));
            if !sub_path.exists() {
                break;
            }
            let sub_name = read_sysfs_string(&sub_path.join("name"));
            let sub_energy = read_sysfs_u64(&sub_path.join("energy_uj")).unwrap_or(0);
            if !sub_name.is_empty() {
                domains.push(RaplDomain {
                    name: sub_name,
                    power_w: 0.0,
                    energy_uj: sub_energy,
                });
            }
            sub_idx += 1;
            if sub_idx > 20 {
                break;
            }
        }
    }

    domains
}

/// Detect CPU TDP. Strategy:
/// 1. Read RAPL `constraint_0_max_power_uj` (PL1 long-term power limit ≈ TDP).
/// 2. Fall back to a heuristic based on logical core count.
pub fn detect_tdp_w() -> f32 {
    let base = Path::new("/sys/class/powercap");
    if let Ok(entries) = fs::read_dir(base) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.starts_with("intel-rapl") && !name.starts_with("amd_rapl") {
                continue;
            }
            for cname in &["constraint_0_max_power_uj", "constraint_1_max_power_uj"] {
                let p = entry.path().join(cname);
                if let Some(max_uj) = read_sysfs_u64(&p) {
                    let w = max_uj as f32 / 1_000_000.0;
                    // Sanity-check: real TDPs are between 5W and 500W
                    if w > 5.0 && w < 500.0 {
                        return w;
                    }
                }
            }
        }
    }

    // Heuristic fallback
    let cores = read_num_cores();
    match cores {
        0..=2 => 15.0, // ultra-low-power / mobile
        3..=4 => 45.0, // mobile / low-end desktop
        5..=6 => 65.0, // mainstream desktop
        7..=8 => 95.0, // high-end desktop
        _ => 125.0,    // workstation / HEDT
    }
}

/// Compute instantaneous power (watts) from two energy counter snapshots.
///
/// RAPL counters are monotonic accumulators in microjoules.
/// Power = Δenergy / Δtime.
///
/// `saturating_sub` handles 32-bit counter wraparound gracefully
/// (yields 0 W for the one affected interval instead of a bogus spike).
pub fn compute_rapl_power(prev: &[RaplDomain], cur: &[RaplDomain], dt_secs: f64) -> CpuPower {
    let mut cpu = CpuPower {
        available: !cur.is_empty(),
        ..Default::default()
    };

    for cur_domain in cur {
        // Match by name across snapshots
        let prev_energy = prev
            .iter()
            .find(|d| d.name == cur_domain.name)
            .map(|d| d.energy_uj)
            .unwrap_or(0);

        let delta_uj = cur_domain.energy_uj.saturating_sub(prev_energy) as f64;
        let power_w = if dt_secs > 0.0 {
            (delta_uj / 1_000_000.0 / dt_secs) as f32
        } else {
            0.0
        };

        let domain = RaplDomain {
            name: cur_domain.name.clone(),
            power_w,
            energy_uj: cur_domain.energy_uj,
        };

        if domain.name.contains("package") || domain.name.contains("Package") {
            cpu.package_w = power_w;
            // Package domain already includes all sub-domains (core + dram + uncore),
            // so total_w = package_w rather than the sum of all domains.
            cpu.total_w = power_w;
        }
        cpu.domains.push(domain);
    }

    cpu
}

// ── CPU utilization ──────────────────────────────────────────────────────────

/// Read per-core tick counts from `/proc/stat`.
///
/// Returns one `(used_ticks, total_ticks)` per logical CPU.
/// "used" = user + nice + system (everything except idle/iowait/steal).
pub fn read_cpu_ticks() -> Vec<(u64, u64)> {
    let Ok(content) = fs::read_to_string("/proc/stat") else {
        return Vec::new();
    };

    content
        .lines()
        // Per-core lines start with "cpuN" (skip the aggregate "cpu " line)
        .filter(|l| l.starts_with("cpu") && !l.starts_with("cpu "))
        .filter_map(|l| {
            let parts: Vec<&str> = l.split_whitespace().collect();
            if parts.len() < 5 {
                return None;
            }
            let user: u64 = parts[1].parse().ok()?;
            let nice: u64 = parts[2].parse().ok()?;
            let system: u64 = parts[3].parse().ok()?;
            let idle: u64 = parts[4].parse().ok()?;
            // Extended fields: iowait, irq, softirq, steal, guest, guest_nice
            let mut total = user + nice + system + idle;
            for p in parts.iter().skip(5) {
                total += p.parse::<u64>().unwrap_or(0);
            }
            let used = user + nice + system;
            Some((used, total))
        })
        .collect()
}

/// Compute per-core utilization % from two tick snapshots.
pub fn compute_cpu_usage(prev: &[(u64, u64)], cur: &[(u64, u64)]) -> Vec<f32> {
    cur.iter()
        .zip(prev.iter())
        .map(|((cu, ct), (pu, pt))| {
            let dt = ct.saturating_sub(*pt);
            let du = cu.saturating_sub(*pu);
            if dt > 0 {
                (du as f32 / dt as f32 * 100.0).clamp(0.0, 100.0)
            } else {
                0.0
            }
        })
        .collect()
}

// ── CPU frequency ────────────────────────────────────────────────────────────

/// Read per-core frequency (MHz) from `/sys/devices/system/cpu/cpuN/cpufreq/`.
pub fn read_cpu_freqs() -> Vec<u32> {
    let mut freqs = Vec::new();
    let mut idx = 0;
    loop {
        let path = format!(
            "/sys/devices/system/cpu/cpu{}/cpufreq/scaling_cur_freq",
            idx
        );
        if !Path::new(&path).exists() {
            break;
        }
        let freq_khz = read_sysfs_u64(Path::new(&path)).unwrap_or(0);
        freqs.push((freq_khz / 1000) as u32);
        idx += 1;
        if idx > 256 {
            break;
        }
    }
    freqs
}

// ── CPU identification ──────────────────────────────────────────────────────

/// Read CPU model name from `/proc/cpuinfo`.
pub fn read_cpu_model() -> String {
    let Ok(content) = fs::read_to_string("/proc/cpuinfo") else {
        return "Unknown CPU".into();
    };
    content
        .lines()
        .find(|l| l.starts_with("model name"))
        .and_then(|l| l.split(':').nth(1))
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "Unknown CPU".into())
}

/// Count logical cores from `/proc/cpuinfo` (lines starting with "processor").
pub fn read_num_cores() -> usize {
    let Ok(content) = fs::read_to_string("/proc/cpuinfo") else {
        return 0;
    };
    content
        .lines()
        .filter(|l| l.starts_with("processor"))
        .count()
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn read_sysfs_string(path: &Path) -> String {
    fs::read_to_string(path)
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn read_sysfs_u64(path: &Path) -> Option<u64> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}
