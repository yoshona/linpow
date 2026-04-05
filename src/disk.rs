//! Disk I/O rates from `/proc/diskstats` and SSD model detection.

use crate::types::DiskInfo;
use std::fs;

/// Read aggregate disk byte counters (read + write) across all physical disks.
///
/// `/proc/diskstats` fields (1-indexed): field 5 = sectors read, field 9 = sectors written.
/// Each sector is 512 bytes. Partitions are filtered out.
pub fn read_disk_counters() -> (u64, u64) {
    let Ok(content) = fs::read_to_string("/proc/diskstats") else {
        return (0, 0);
    };

    let mut total_read = 0u64;
    let mut total_write = 0u64;

    for line in content.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 14 {
            continue;
        }
        let dev_name = parts[2];
        if !is_physical_disk(dev_name) {
            continue;
        }
        let sectors_read: u64 = parts[5].parse().unwrap_or(0);
        let sectors_written: u64 = parts[9].parse().unwrap_or(0);
        total_read += sectors_read * 512;
        total_write += sectors_written * 512;
    }

    (total_read, total_write)
}

/// Compute disk I/O rates (bytes/sec) from two counter snapshots.
pub fn compute_disk_rates(prev: (u64, u64), cur: (u64, u64), dt_secs: f64) -> DiskInfo {
    let read_rate = if dt_secs > 0.0 {
        cur.0.saturating_sub(prev.0) as f64 / dt_secs
    } else {
        0.0
    };
    let write_rate = if dt_secs > 0.0 {
        cur.1.saturating_sub(prev.1) as f64 / dt_secs
    } else {
        0.0
    };

    let disk_name = detect_disk_model();

    DiskInfo {
        read_bytes_per_sec: read_rate,
        write_bytes_per_sec: write_rate,
        disk_name,
    }
}

/// Filter: keep only whole physical disks, skip partitions.
///
/// - `sdX` → keep, `sdX1` → skip
/// - `nvme0n1` → keep, `nvme0n1p1` → skip
/// - `mmcblk0` → keep, `mmcblk0p1` → skip
fn is_physical_disk(name: &str) -> bool {
    if name.starts_with("sd") {
        let suffix = &name[2..];
        suffix.is_empty() || !suffix.chars().any(|c| c.is_ascii_digit())
    } else if name.starts_with("nvme") {
        // nvme0n1 (keep), nvme0n1p1 (skip)
        !name.contains('p') || name.matches('p').count() == 0
    } else if name.starts_with("mmcblk") {
        let rest = &name[6..];
        !rest.contains('p')
    } else {
        false
    }
}

/// Detect the model name of the first physical disk from `/sys/block/`.
fn detect_disk_model() -> String {
    let Ok(entries) = fs::read_dir("/sys/block") else {
        return String::new();
    };

    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with("sd") || name.starts_with("nvme") || name.starts_with("mmcblk") {
            let model_path = format!("/sys/block/{}/device/model", name);
            if let Ok(model) = fs::read_to_string(&model_path) {
                let model = model.trim().to_string();
                if !model.is_empty() {
                    return model;
                }
            }
        }
    }

    String::new()
}
