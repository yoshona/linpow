# linpow

Real-time power consumption monitor for Linux, with an interactive TUI and JSON output mode.

## Features

- **CPU** — RAPL power (package / core / dram / uncore), per-core utilization, frequency, temperature
- **GPU** — NVIDIA (`nvidia-smi`) / AMD / Intel: power, utilization, VRAM, temperature
- **Battery** — Charge/discharge power, time remaining, health, cycle count
- **Fans** — RPM and estimated power draw (cubic model)
- **Disk** — Read/write throughput, SSD power estimation
- **Network** — Per-interface rates, Wi-Fi SSID / signal strength
- **Processes** — Top N by RSS, CPU usage, disk I/O
- **Cumulative energy** — Real-time Wh integration per component
- **JSON output** — `--json` streaming output for scripting

## Build

```bash
cargo build --release
```

## Usage

```bash
# Interactive TUI
linpow

# Custom sampling interval (ms)
linpow --interval 500

# Streaming JSON output
linpow --json
linpow --json --interval 1000 > metrics.jsonl
```

## Key Bindings

| Key                      | Action                                           |
|--------------------------|--------------------------------------------------|
| `q` / `Esc`              | Quit                                             |
| `↑` `↓` / `j` `k`        | Move up / down                                   |
| `←` / `h`                | Collapse / jump to parent                        |
| `→`                      | Expand                                           |
| `-`                      | Collapse all                                     |
| `+` / `=`                | Expand all                                       |
| `Space`                  | Pin / unpin chart                                |
| `a`                      | Cycle SMA window (off → 5s → 10s)                |
| `l`                      | Cycle polling interval (250ms → 500ms → 1s → 2s) |
| `r`                      | Reset cumulative energy and history              |
| `Home` / `PgUp` / `PgDn` | Fast navigation                                  |

Mouse: left-click to select / toggle, scroll wheel to navigate.

## Layout

```
┌─ Power Tree ────────────────────────────────────────────┐
│ Component            Freq      Temp       Power  Cumul. │
│                                                         │
│ Battery 95% (3h 25m remaining)         -12.345 W  ...Wh │
│ ─────────────────────────────────────────────────────── │
│ CPU (8 cores, 35%)                    45.230 W  ...Wh   │
│ │  ├─ core                              32.100 W        │
│ │  ├─ dram                               8.500 W        │
│ │  ├─ cpu0  (12.5%) ▓░░░░░░░░░          ...  2400 MHz   │
│ │  ├─ cpu1  ( 5.0%) ░░░░░░░░░░          ...  2400 MHz   │
│ │  └─ ...                                               │
│ ─────────────────────────────────────────────────────── │
│ GPU (15%)                             25.000 W          │
│ Disk (nvme0n1)                        ≈0.150 W          │
│ Fans                                  ≈1.200 W          │
│ Network                               ↓1.2 MB/s ↑0.0    │
│ Software (top 10 by RSS, 2.4 GB)                        │
├─────────────────────────────────────────────────────────┤
│ CPU — 45.230 W                                          │
│ ██████████████▁▁▁▁▁▁▁                                   │
├─ q quit  r reset  a avg:0s  l 250ms ── ⏱ 02m 15s ──────┤
└─────────────────────────────────────────────────────────┘
```

- **Power column**: RAPL readings are exact; estimates are prefixed with `≈`
- **History column**: Sparkline shown when terminal width > 90
- **Charts**: Bar charts for power/memory history of selected or pinned items
- **Footer**: Key hints + power color legend + session uptime

## Power Measurement Methods

| Component    | Method                                                                   |
|--------------|--------------------------------------------------------------------------|
| CPU          | RAPL (`/sys/class/powercap/`) energy counter deltas / TDP-based fallback |
| GPU (NVIDIA) | `nvidia-smi`                                                             |
| GPU (AMD)    | sysfs `power1_input`                                                     |
| SSD          | Linear model: 0.03W idle + up to 2.5W under load                         |
| Fans         | Cubic model: `(rpm / max_rpm)³` watts                                    |
| Battery      | `voltage_now × current_now` or `power_now` from sysfs                    |

CPU TDP is auto-detected from RAPL `constraint_0_max_power_uj`, with a core-count heuristic fallback.

## Requirements

- Linux with sysfs / procfs
- RAPL support for accurate CPU power (Intel / AMD)
- `nvidia-smi` in PATH for NVIDIA GPU power
- No root required

## License

MIT
