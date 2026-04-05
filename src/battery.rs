//! Battery and AC adapter status from `/sys/class/power_supply/`.

use crate::types::{AdapterInfo, BatteryInfo};
use std::fs;
use std::path::Path;

/// Read the first detected battery's status.
///
/// sysfs exposes voltages in µV, currents in µA, and energies in µWh.
/// We convert to mV, mA, and Wh respectively for display.
pub fn read_battery() -> BatteryInfo {
    let base = Path::new("/sys/class/power_supply");
    if !base.exists() {
        return BatteryInfo::default();
    }

    let Ok(entries) = fs::read_dir(base) else {
        return BatteryInfo::default();
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let p_type = read_sysfs_string(&path.join("type"));
        if p_type != "Battery" {
            continue;
        }

        let present = read_sysfs_int(&path.join("present")).unwrap_or(0) != 0;
        if !present {
            continue;
        }

        let status = read_sysfs_string(&path.join("status"));
        let voltage_now = read_sysfs_int(&path.join("voltage_now")).unwrap_or(0) as f64 / 1000.0; // µV → mV
        let current_now = read_sysfs_int(&path.join("current_now")).unwrap_or(0) as f64 / 1000.0; // µA → mA
        let energy_now = read_sysfs_int(&path.join("energy_now")).unwrap_or(0) as f64 / 1_000_000.0; // µWh → Wh
        let energy_full =
            read_sysfs_int(&path.join("energy_full")).unwrap_or(0) as f64 / 1_000_000.0;
        let energy_full_design =
            read_sysfs_int(&path.join("energy_full_design")).unwrap_or(0) as f64 / 1_000_000.0;
        let capacity = read_sysfs_int(&path.join("capacity")).unwrap_or(0) as f64;
        let cycle_count = read_sysfs_int(&path.join("cycle_count")).unwrap_or(-1);

        // Power = V × I (mV × mA = mW → W via /1_000_000).
        // Fall back to the kernel's power_now if V/I aren't available.
        let power_w = if voltage_now > 0.0 && current_now != 0.0 {
            (voltage_now * current_now / 1_000_000.0).abs()
        } else {
            read_sysfs_int(&path.join("power_now")).unwrap_or(0) as f64 / 1_000_000.0
        };

        // Sign convention: positive = discharging (draining battery), negative = charging.
        let drain_w = if status == "Discharging" {
            power_w
        } else if status == "Charging" {
            -power_w
        } else {
            0.0
        };

        let health_pct = if energy_full_design > 0.0 {
            (energy_full / energy_full_design * 100.0).clamp(0.0, 100.0)
        } else {
            100.0
        };

        return BatteryInfo {
            present: true,
            status,
            percent: capacity,
            voltage_mv: voltage_now,
            current_ma: current_now,
            power_w,
            energy_now_wh: energy_now,
            energy_full_wh: energy_full,
            drain_w,
            time_remaining_min: 0,
            external_connected: false,
            cycle_count,
            health_pct,
        };
    }

    BatteryInfo::default()
}

/// Read AC adapter (Mains) online status.
pub fn read_adapter() -> AdapterInfo {
    let base = Path::new("/sys/class/power_supply");
    if !base.exists() {
        return AdapterInfo::default();
    }

    let Ok(entries) = fs::read_dir(base) else {
        return AdapterInfo::default();
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let p_type = read_sysfs_string(&path.join("type"));
        if p_type == "Mains" {
            let online = read_sysfs_int(&path.join("online")).unwrap_or(0) != 0;
            return AdapterInfo { online };
        }
    }

    AdapterInfo::default()
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
