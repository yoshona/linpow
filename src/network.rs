//! Network interface rates from `/proc/net/dev` and Wi-Fi info from sysfs.

use crate::types::InterfaceRate;
use std::collections::HashMap;
use std::fs;

/// Read byte counters (rx, tx) per interface from `/proc/net/dev`.
/// The loopback interface is excluded.
pub fn read_net_counters() -> HashMap<String, (u64, u64)> {
    let mut counters = HashMap::new();
    let Ok(content) = fs::read_to_string("/proc/net/dev") else {
        return counters;
    };

    for line in content.lines().skip(2) {
        let Some((iface, data)) = line.split_once(':') else {
            continue;
        };
        let iface = iface.trim();
        if iface == "lo" {
            continue;
        }
        let parts: Vec<&str> = data.split_whitespace().collect();
        if parts.len() < 10 {
            continue;
        }
        // Field 0 = rx_bytes, Field 8 = tx_bytes (see `man 5 /proc/net/dev`)
        let rx_bytes: u64 = parts[0].parse().unwrap_or(0);
        let tx_bytes: u64 = parts[8].parse().unwrap_or(0);
        counters.insert(iface.to_string(), (rx_bytes, tx_bytes));
    }

    counters
}

/// Compute per-interface byte rates (B/s) from two counter snapshots.
/// Also detects Wi-Fi vs Ethernet and reads link speed / RSSI.
pub fn compute_net_rates(
    prev: &HashMap<String, (u64, u64)>,
    cur: &HashMap<String, (u64, u64)>,
    dt_secs: f64,
) -> Vec<InterfaceRate> {
    let mut rates = Vec::new();

    for (iface, (cur_rx, cur_tx)) in cur {
        let (prev_rx, prev_tx) = prev.get(iface).copied().unwrap_or((0, 0));
        let rx_rate = if dt_secs > 0.0 {
            cur_rx.saturating_sub(prev_rx) as f64 / dt_secs
        } else {
            0.0
        };
        let tx_rate = if dt_secs > 0.0 {
            cur_tx.saturating_sub(prev_tx) as f64 / dt_secs
        } else {
            0.0
        };

        let is_wifi = is_wireless(iface);
        let link_speed = if is_wifi {
            0 // Wi-Fi link speed requires iw, not exposed via sysfs
        } else {
            read_link_speed(iface)
        };

        let ssid = if is_wifi {
            read_wifi_ssid(iface)
        } else {
            String::new()
        };

        let rssi = if is_wifi { read_wifi_rssi(iface) } else { 0 };

        rates.push(InterfaceRate {
            name: iface.clone(),
            bytes_in_per_sec: rx_rate,
            bytes_out_per_sec: tx_rate,
            is_wifi,
            link_speed_mbps: link_speed,
            wifi_ssid: ssid,
            wifi_rssi_dbm: rssi,
        });
    }

    // Sort by download rate descending
    rates.sort_by(|a, b| {
        b.bytes_in_per_sec
            .partial_cmp(&a.bytes_in_per_sec)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    rates
}

/// Check if an interface is wireless by looking for `/sys/class/net/<iface>/wireless`.
fn is_wireless(iface: &str) -> bool {
    let wireless_path = format!("/sys/class/net/{}/wireless", iface);
    std::path::Path::new(&wireless_path).exists()
}

/// Read Ethernet link speed (Mbps) from `/sys/class/net/<iface>/speed`.
fn read_link_speed(iface: &str) -> u32 {
    let speed_path = format!("/sys/class/net/{}/speed", iface);
    fs::read_to_string(&speed_path)
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

/// Read WiFi SSID. Requires `iw` or `nmcli` — placeholder for now.
fn read_wifi_ssid(_iface: &str) -> String {
    String::new()
}

/// Read Wi-Fi signal strength (dBm) from `/proc/net/wireless`.
fn read_wifi_rssi(_iface: &str) -> i32 {
    let Ok(content) = fs::read_to_string("/proc/net/wireless") else {
        return 0;
    };
    // Format: "iface: status link level noise ..."
    for line in content.lines().skip(2) {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 4 {
            if let Ok(level) = parts[3].parse::<f32>() {
                return level as i32;
            }
        }
    }
    0
}
