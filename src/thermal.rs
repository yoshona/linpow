//! Temperature sensors and fan speeds from `/sys/class/hwmon/` (and `/sys/class/thermal/` as fallback).

use crate::types::{FanInfo, TempSensor};
use std::fs;
use std::path::Path;

/// Read all temperature sensors from hwmon, grouped into categories for the TUI.
pub fn read_temperatures() -> Vec<TempSensor> {
    let mut sensors = Vec::new();
    let base = Path::new("/sys/class/hwmon");
    if !base.exists() {
        return read_thermal_zones(); // Fallback for systems without hwmon
    }

    let Ok(entries) = fs::read_dir(base) else {
        return sensors;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let hwmon_name = read_sysfs_string(&path.join("name"));

        // Iterate tempN_input files (1-indexed)
        let mut idx = 1;
        loop {
            let input_path = path.join(format!("temp{}_input", idx));
            if !input_path.exists() {
                break;
            }

            let label = read_sysfs_string(&path.join(format!("temp{}_label", idx)));
            let raw_temp = read_sysfs_int(&input_path).unwrap_or(0);
            let temp_c = raw_temp as f32 / 1000.0; // millidegrees → °C

            let category = categorize_temp(&hwmon_name, &label);

            sensors.push(TempSensor {
                label: if label.is_empty() {
                    format!("{}_temp{}", hwmon_name, idx)
                } else {
                    label
                },
                category,
                value_celsius: temp_c,
            });

            idx += 1;
            if idx > 50 {
                break;
            }
        }
    }

    sensors
}

/// Fallback: read from `/sys/class/thermal/thermal_zone*` when hwmon is absent.
fn read_thermal_zones() -> Vec<TempSensor> {
    let mut sensors = Vec::new();
    let base = Path::new("/sys/class/thermal");
    if !base.exists() {
        return sensors;
    }

    let Ok(entries) = fs::read_dir(base) else {
        return sensors;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with("thermal_zone") {
            continue;
        }

        let tz_type = read_sysfs_string(&path.join("type"));
        let raw_temp = read_sysfs_int(&path.join("temp")).unwrap_or(0);
        let temp_c = raw_temp as f32 / 1000.0;

        sensors.push(TempSensor {
            label: tz_type.clone(),
            category: categorize_temp(&tz_type, &tz_type),
            value_celsius: temp_c,
        });
    }

    sensors
}

/// Read fan speeds and estimate power draw.
///
/// Power model: cubic relationship — `(rpm / max_rpm)³` watts.
/// At full speed a typical fan draws ~1W.
pub fn read_fans() -> Vec<FanInfo> {
    let mut fans = Vec::new();
    let base = Path::new("/sys/class/hwmon");
    if !base.exists() {
        return fans;
    }

    let Ok(entries) = fs::read_dir(base) else {
        return fans;
    };

    for entry in entries.flatten() {
        let path = entry.path();

        let mut idx = 1;
        loop {
            let input_path = path.join(format!("fan{}_input", idx));
            if !input_path.exists() {
                break;
            }

            let rpm = read_sysfs_int(&input_path).unwrap_or(0) as f32;
            let label = read_sysfs_string(&path.join(format!("fan{}_label", idx)));
            let min_rpm = read_sysfs_int(&path.join(format!("fan{}_min", idx))).unwrap_or(0) as f32;
            let max_rpm = read_sysfs_int(&path.join(format!("fan{}_max", idx))).unwrap_or(0) as f32;

            let max = if max_rpm > 0.0 { max_rpm } else { 5000.0 };
            let estimated_power = (rpm / max).powi(3);

            fans.push(FanInfo {
                name: if label.is_empty() {
                    format!("fan{}", idx)
                } else {
                    label
                },
                rpm,
                min_rpm,
                max_rpm: max,
                estimated_power_w: estimated_power,
            });

            idx += 1;
            if idx > 20 {
                break;
            }
        }
    }

    fans
}

/// Classify a temperature sensor into a category based on hwmon name and label.
fn categorize_temp(hwmon_name: &str, label: &str) -> String {
    let combined = format!("{} {}", hwmon_name, label).to_lowercase();

    if combined.contains("cpu")
        || combined.contains("core")
        || combined.contains("package")
        || combined.contains("tctl")
        || combined.contains("k10temp")
        || combined.contains("coretemp")
    {
        "CPU".into()
    } else if combined.contains("gpu")
        || combined.contains("nvme")
        || combined.contains("amdgpu")
        || combined.contains("radeon")
    {
        "GPU".into()
    } else if combined.contains("dram")
        || combined.contains("memory")
        || combined.contains("sodimm")
    {
        "Memory".into()
    } else if combined.contains("ssd") || combined.contains("nvme") || combined.contains("disk") {
        "SSD".into()
    } else if combined.contains("bat") || combined.contains("battery") {
        "Battery".into()
    } else if combined.contains("board")
        || combined.contains("motherboard")
        || combined.contains("pch")
    {
        "Board".into()
    } else {
        "Other".into()
    }
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

/// Read per-core CPU temperatures by matching hwmon labels like "Core 0", "Core 1".
/// Returns a Vec indexed by core number, with temperature in Celsius.
pub fn read_core_temps(num_cores: usize) -> Vec<f32> {
    let mut core_temps = vec![0.0f32; num_cores];
    if num_cores == 0 {
        return core_temps;
    }

    let base = Path::new("/sys/class/hwmon");
    let Ok(entries) = fs::read_dir(base) else {
        return core_temps;
    };

    for entry in entries.flatten() {
        let path = entry.path();

        let mut idx = 1;
        loop {
            let input_path = path.join(format!("temp{}_input", idx));
            if !input_path.exists() {
                break;
            }

            let label = read_sysfs_string(&path.join(format!("temp{}_label", idx)));
            let raw_temp = read_sysfs_int(&input_path).unwrap_or(0);
            let temp_c = raw_temp as f32 / 1000.0;

            if let Some(core_num) = extract_core_number(&label.to_lowercase()) {
                if (core_num as usize) < num_cores {
                    core_temps[core_num as usize] = temp_c;
                }
            }

            idx += 1;
            if idx > 100 {
                break;
            }
        }
    }

    core_temps
}

/// Extract the core index from labels like "Core 0", "CPU Core 3", etc.
fn extract_core_number(label: &str) -> Option<u32> {
    let parts: Vec<&str> = label.split_whitespace().collect();
    for i in 0..parts.len().saturating_sub(1) {
        if parts[i] == "core" {
            if let Ok(n) = parts[i + 1].parse::<u32>() {
                return Some(n);
            }
        }
    }
    None
}
