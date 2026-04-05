//! Per-process CPU, memory, and disk I/O metrics from `/proc/`.
//!
//! Reads `/proc/[pid]/stat` for CPU ticks, `/proc/[pid]/status` for RSS,
//! and `/proc/[pid]/io` for disk counters. CPU % is htop-style (100% = one
//! core fully utilized).

use crate::types::ProcessInfo;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::time::Instant;

/// Read top processes sorted by RSS.
///
/// Returns the top `max_count` processes and a tick map to carry forward
/// for rate computation on the next call.
///
/// The tick map stores `(utime, stime, read_bytes, write_bytes, timestamp)`
/// per PID so we can compute deltas on each call.
pub fn read_top_processes(
    prev_ticks: &HashMap<i32, (u64, u64, u64, u64, Instant)>,
    num_cpus: usize,
    max_count: usize,
) -> (
    Vec<ProcessInfo>,
    HashMap<i32, (u64, u64, u64, u64, Instant)>,
) {
    let mut processes = Vec::new();
    let mut new_ticks = HashMap::new();

    let Ok(entries) = fs::read_dir("/proc") else {
        return (processes, new_ticks);
    };

    let now = Instant::now();

    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        let Ok(pid) = name_str.parse::<i32>() else {
            continue; // Not a numeric PID directory
        };

        let proc_path = Path::new("/proc").join(name_str.as_ref());

        // Parse /proc/[pid]/stat
        let Ok(stat_content) = fs::read_to_string(proc_path.join("stat")) else {
            continue;
        };

        let Some(proc_name) = extract_name(&stat_content) else {
            continue;
        };
        let Some(ticks) = extract_ticks(&stat_content) else {
            continue;
        };

        // Disk I/O counters from /proc/[pid]/io
        let (read_bytes, write_bytes) = read_proc_io(&proc_path);

        // RSS from /proc/[pid]/status
        let rss_mb = read_rss_mb(&proc_path);

        // CPU %: delta(ticks) / delta(wall_time)
        // On Linux, USER_HZ = 100, so tick / second = percentage directly.
        let cpu_pct = if let Some((prev_utime, prev_stime, _, _, prev_time)) = prev_ticks.get(&pid)
        {
            let dt_real = now.duration_since(*prev_time).as_secs_f64();
            let dt_cpu = (ticks.0 + ticks.1).saturating_sub(*prev_utime + *prev_stime) as f64;
            if dt_real > 0.0 {
                (dt_cpu / dt_real).clamp(0.0, num_cpus as f64 * 100.0) as f32
            } else {
                0.0
            }
        } else {
            0.0
        };

        // Disk rate deltas
        let (disk_read_rate, disk_write_rate) =
            if let Some((_, _, prev_read, prev_write, prev_time)) = prev_ticks.get(&pid) {
                let dt = now.duration_since(*prev_time).as_secs_f64();
                let dr = if dt > 0.0 {
                    read_bytes.saturating_sub(*prev_read) as f64 / dt
                } else {
                    0.0
                };
                let dw = if dt > 0.0 {
                    write_bytes.saturating_sub(*prev_write) as f64 / dt
                } else {
                    0.0
                };
                (dr, dw)
            } else {
                (0.0, 0.0)
            };

        new_ticks.insert(pid, (ticks.0, ticks.1, read_bytes, write_bytes, now));

        // Filter out idle processes to keep the list manageable
        if cpu_pct > 0.01 || rss_mb > 10.0 {
            processes.push(ProcessInfo {
                pid,
                name: proc_name,
                cpu_pct,
                rss_mb,
                disk_read_bytes_per_sec: disk_read_rate,
                disk_write_bytes_per_sec: disk_write_rate,
                disk_read_bytes: read_bytes,
                disk_write_bytes: write_bytes,
                alive: true,
            });
        }
    }

    // Sort by RSS descending, keep top N
    processes.sort_by(|a, b| {
        b.rss_mb
            .partial_cmp(&a.rss_mb)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    processes.truncate(max_count);

    (processes, new_ticks)
}

/// Extract the comm name from `/proc/[pid]/stat`.
/// The name is in parentheses and may contain spaces, so we find the
/// first `(` and last `)` rather than splitting by spaces.
fn extract_name(stat: &str) -> Option<String> {
    let start = stat.find('(')?;
    let end = stat.rfind(')')?;
    Some(stat[start + 1..end].to_string())
}

/// Extract utime and stime from `/proc/[pid]/stat` (fields 14 and 15).
///
/// After the closing `)`, fields are: state (field 3), ppid, ..., utime
/// is the 12th field after `)` (0-indexed: 11).
fn extract_ticks(stat: &str) -> Option<(u64, u64)> {
    let end = stat.rfind(')')?;
    let fields: Vec<&str> = stat[end + 1..].split_whitespace().collect();
    let utime: u64 = fields.get(11)?.parse().ok()?;
    let stime: u64 = fields.get(12)?.parse().ok()?;
    Some((utime, stime))
}

/// Read `read_bytes` and `write_bytes` from `/proc/[pid]/io`.
fn read_proc_io(proc_path: &Path) -> (u64, u64) {
    let Ok(content) = fs::read_to_string(proc_path.join("io")) else {
        return (0, 0);
    };
    let mut read_bytes = 0u64;
    let mut write_bytes = 0u64;
    for line in content.lines() {
        if let Some(val) = line.strip_prefix("read_bytes:") {
            read_bytes = val.trim().parse().unwrap_or(0);
        } else if let Some(val) = line.strip_prefix("write_bytes:") {
            write_bytes = val.trim().parse().unwrap_or(0);
        }
    }
    (read_bytes, write_bytes)
}

/// Read VmRSS (resident set size) from `/proc/[pid]/status`, converted to MiB.
fn read_rss_mb(proc_path: &Path) -> f64 {
    let Ok(content) = fs::read_to_string(proc_path.join("status")) else {
        return 0.0;
    };
    for line in content.lines() {
        if let Some(val) = line.strip_prefix("VmRSS:") {
            let kb: f64 = val
                .trim()
                .split_whitespace()
                .next()
                .unwrap_or("0")
                .parse()
                .unwrap_or(0.0);
            return kb / 1024.0;
        }
    }
    0.0
}
