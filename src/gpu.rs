//! GPU detection and metrics for NVIDIA, AMD, and Intel.
//!
//! Detection priority: NVIDIA (nvidia-smi) → AMD (sysfs) → Intel (sysfs).
//! Only the first successfully detected GPU is reported.

use crate::types::{GpuInfo, GpuVendor};
use std::fs;
use std::path::Path;
use std::process::Command;

/// Detect and read GPU info. Tries vendors in priority order.
pub fn read_gpu_info() -> GpuInfo {
    if let Some(info) = read_nvidia_gpu() {
        return info;
    }
    if let Some(info) = read_amd_gpu() {
        return info;
    }
    if let Some(info) = read_intel_gpu() {
        return info;
    }
    GpuInfo::default()
}

/// NVIDIA: query `nvidia-smi` for power, utilization, VRAM, temperature, and clocks.
fn read_nvidia_gpu() -> Option<GpuInfo> {
    let output = Command::new("nvidia-smi")
        .args([
            "--query-gpu=name,power.draw,utilization.gpu,memory.used,memory.total,temperature.gpu,clocks.current.graphics",
            "--format=csv,noheader,nounits",
        ])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let line = String::from_utf8_lossy(&output.stdout);
    let parts: Vec<&str> = line.trim().split(',').collect();
    if parts.len() < 7 {
        return None;
    }

    Some(GpuInfo {
        available: true,
        vendor: GpuVendor::Nvidia,
        name: parts[0].trim().to_string(),
        power_w: parts[1].trim().parse().unwrap_or(0.0),
        utilization_pct: parts[2].trim().parse().unwrap_or(0),
        memory_used_mb: parts[3].trim().parse().unwrap_or(0),
        memory_total_mb: parts[4].trim().parse().unwrap_or(0),
        temperature_c: parts[5].trim().parse().unwrap_or(0.0),
        core_clock_mhz: parts[6].trim().parse().unwrap_or(0),
    })
}

/// AMD: read from `/sys/class/drm/` + hwmon.
///
/// Identifies AMD GPUs by PCI vendor ID `0x1002`.
fn read_amd_gpu() -> Option<GpuInfo> {
    let drm_dir = Path::new("/sys/class/drm");
    let Ok(entries) = fs::read_dir(drm_dir) else {
        return None;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let device_path = path.join("device");

        let vendor_path = device_path.join("vendor");
        let vendor_id = read_sysfs_string(&vendor_path);
        if vendor_id != "0x1002" {
            continue; // Not an AMD device
        }

        let name = read_sysfs_string(&device_path.join("product_name"));
        let name = if name.is_empty() {
            "AMD GPU".to_string()
        } else {
            name
        };

        let utilization = read_sysfs_int(&device_path.join("gpu_busy_percent")).unwrap_or(0) as u32;

        // Power from HWMON (microwatts → watts)
        let mut power_w = 0.0f32;
        if let Ok(hwmon_entries) = fs::read_dir(device_path.join("hwmon")) {
            for hwmon in hwmon_entries.flatten() {
                let hp = hwmon.path();
                if let Some(pw) = read_sysfs_int(&hp.join("power1_input")) {
                    power_w = pw as f32 / 1_000_000.0;
                }
            }
        }

        // VRAM (bytes → MiB)
        let mem_used = read_sysfs_int(&device_path.join("mem_info_vram_used")).unwrap_or(0) as u64
            / (1024 * 1024);
        let mem_total = read_sysfs_int(&device_path.join("mem_info_vram_total")).unwrap_or(0)
            as u64
            / (1024 * 1024);

        // Temperature from HWMON (millidegrees → °C)
        let mut temp = 0.0f32;
        if let Ok(hwmon_entries) = fs::read_dir(device_path.join("hwmon")) {
            for hwmon in hwmon_entries.flatten() {
                let hp = hwmon.path();
                if let Some(t) = read_sysfs_int(&hp.join("temp1_input")) {
                    temp = t as f32 / 1000.0;
                }
            }
        }

        // Clock parsing from pp_dpm_sclk is complex; report 0 for now.
        let clock = 0u32;

        return Some(GpuInfo {
            available: true,
            vendor: GpuVendor::Amd,
            name,
            power_w,
            utilization_pct: utilization,
            memory_used_mb: mem_used,
            memory_total_mb: mem_total,
            temperature_c: temp,
            core_clock_mhz: clock,
        });
    }

    None
}

/// Intel: read from `/sys/class/drm/` (i915).
///
/// Identifies Intel GPUs by PCI vendor ID `0x8086`.
/// Intel doesn't expose power directly via sysfs.
fn read_intel_gpu() -> Option<GpuInfo> {
    let drm_dir = Path::new("/sys/class/drm");
    let Ok(entries) = fs::read_dir(drm_dir) else {
        return None;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let device_path = path.join("device");
        let vendor_path = device_path.join("vendor");
        let vendor_id = read_sysfs_string(&vendor_path);

        if vendor_id != "0x8086" {
            continue; // Not an Intel device
        }

        // Use gt_cur_freq_mhz as a signal that the i915 driver is active.
        let gt_freq = read_sysfs_int(&device_path.join("gt_cur_freq_mhz")).unwrap_or(0) as u32;
        if gt_freq == 0 {
            continue;
        }

        let rp0_freq = read_sysfs_int(&device_path.join("gt_RP0_freq_mhz")).unwrap_or(0) as u32;

        return Some(GpuInfo {
            available: true,
            vendor: GpuVendor::Intel,
            name: "Intel GPU".to_string(),
            power_w: 0.0,
            utilization_pct: 0,
            memory_used_mb: 0,
            memory_total_mb: 0,
            temperature_c: 0.0,
            core_clock_mhz: gt_freq.max(rp0_freq),
        });
    }

    None
}

fn read_sysfs_string(path: &Path) -> String {
    fs::read_to_string(path)
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn read_sysfs_int(path: &Path) -> Option<i64> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}
