use crate::sma::TimeSma;
use crate::types::*;
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;
use std::collections::{BTreeMap, VecDeque};
use std::time::Instant;
use unicode_width::UnicodeWidthStr;

// ── Styles ───────────────────────────────────────────────────────────────────

const BOLD: Style = Style::new().add_modifier(Modifier::BOLD);
const DIM: Style = Style::new().fg(Color::DarkGray);
const DATA_STYLE: Style = Style::new().fg(Color::Rgb(80, 140, 255));
const PENDING: Style = Style::new().fg(Color::Magenta);
const TREE_STYLE: Style = Style::new().fg(Color::Reset);
const PIN_MARKER: &str = "▸ ";
const HISTORY_LEN: usize = 240;
const CHART_HEIGHT: u16 = 7;

const COL_FREQ: u16 = 10;
const COL_TEMP: u16 = 16;
const COL_CUR: u16 = 14;
const COL_TOT: u16 = 14;

const SPARK_CHARS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '▇'];
const BAR_EIGHTHS: [char; 9] = [' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

fn power_color(w: f32) -> Color {
    match w {
        w if w < 5.0 => Color::Rgb(46, 139, 87),
        w if w < 15.0 => Color::Rgb(220, 180, 0),
        w if w < 30.0 => Color::Rgb(255, 140, 0),
        _ => Color::Rgb(255, 50, 50),
    }
}

fn fmt_wh(wh: f64) -> String {
    let mwh = wh * 1000.0;
    if mwh.abs() >= 1000.0 {
        format!("{:>10.3} Wh", wh)
    } else {
        format!("{:>10.2} mWh", mwh)
    }
}

fn human_rate(bps: f64) -> String {
    match bps {
        b if b < 1024.0 => format!("{:>7.0} B/s", b),
        b if b < 1024.0 * 1024.0 => format!("{:>7.1} KB/s", b / 1024.0),
        b => format!("{:>7.1} MB/s", b / (1024.0 * 1024.0)),
    }
}

fn human_bytes(b: f64) -> String {
    match b {
        b if b < 1024.0 => format!("{:.0} B", b),
        b if b < 1024.0 * 1024.0 => format!("{:.1} KB", b / 1024.0),
        b if b < 1024.0 * 1024.0 * 1024.0 => format!("{:.1} MB", b / (1024.0 * 1024.0)),
        b => format!("{:.2} GB", b / (1024.0 * 1024.0 * 1024.0)),
    }
}

fn fmt_freq(mhz: f32) -> String {
    if mhz > 0.0 {
        format!("{:.0} MHz", mhz)
    } else {
        String::new()
    }
}

fn usage_bar(pct: f32) -> String {
    let width = 10;
    let filled = ((pct / 100.0) * width as f32).round() as usize;
    let empty = width - filled.min(width);
    format!("{}{}", "▓".repeat(filled), "░".repeat(empty))
}

// ── TreeRow ──────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
enum PowerPrefix {
    Exact,
    Estimated,
    #[allow(dead_code)]
    MaxBound,
}

struct TreeRow {
    prefix: String,
    label: String,
    freq: String,
    temp: String,
    current: String,
    total: String,
    label_style: Style,
    current_style: Style,
    key: Option<&'static str>,
    parent: Option<&'static str>,
    pinned: bool,
}

impl TreeRow {
    fn pw(
        key: &'static str,
        parent: Option<&'static str>,
        prefix: &str,
        label: &str,
        watts: f32,
        wh: f64,
        style: Style,
        pinned: bool,
    ) -> Self {
        Self::pw_inner(
            key,
            parent,
            prefix,
            label,
            watts,
            wh,
            "",
            "",
            style,
            pinned,
            PowerPrefix::Exact,
        )
    }

    fn pw_est(
        key: &'static str,
        parent: Option<&'static str>,
        prefix: &str,
        label: &str,
        watts: f32,
        wh: f64,
        style: Style,
        pinned: bool,
    ) -> Self {
        Self::pw_inner(
            key,
            parent,
            prefix,
            label,
            watts,
            wh,
            "",
            "",
            style,
            pinned,
            PowerPrefix::Estimated,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn pw_full(
        key: &'static str,
        parent: Option<&'static str>,
        prefix: &str,
        label: &str,
        watts: f32,
        wh: f64,
        freq: &str,
        temp: &str,
        style: Style,
        pinned: bool,
    ) -> Self {
        Self::pw_inner(
            key,
            parent,
            prefix,
            label,
            watts,
            wh,
            freq,
            temp,
            style,
            pinned,
            PowerPrefix::Exact,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn pw_inner(
        key: &'static str,
        parent: Option<&'static str>,
        prefix: &str,
        label: &str,
        watts: f32,
        wh: f64,
        freq: &str,
        temp: &str,
        style: Style,
        pinned: bool,
        power_prefix: PowerPrefix,
    ) -> Self {
        let w = watts + 0.0;
        let current = match power_prefix {
            PowerPrefix::Exact => format!("{:>7.3} W", w),
            PowerPrefix::Estimated => format!("≈{:.3} W", w),
            PowerPrefix::MaxBound => format!("≤{:.3} W", w),
        };
        let total = match power_prefix {
            PowerPrefix::Exact => fmt_wh(wh),
            PowerPrefix::Estimated => {
                let s = fmt_wh(wh);
                format!("≈{}", s.trim_start())
            }
            PowerPrefix::MaxBound => {
                let s = fmt_wh(wh);
                format!("≤{}", s.trim_start())
            }
        };
        Self {
            prefix: prefix.to_string(),
            label: label.to_string(),
            freq: freq.to_string(),
            temp: temp.to_string(),
            current,
            total,
            label_style: style.fg(style.fg.unwrap_or(power_color(w.abs()))),
            current_style: Style::default().fg(power_color(w.abs())),
            key: Some(key),
            parent,
            pinned,
        }
    }

    fn info(
        parent: Option<&'static str>,
        prefix: &str,
        label: &str,
        current: &str,
        total: &str,
        style: Style,
    ) -> Self {
        Self {
            prefix: prefix.to_string(),
            label: label.to_string(),
            freq: String::new(),
            temp: String::new(),
            current: current.to_string(),
            total: total.to_string(),
            label_style: style,
            current_style: style,
            key: None,
            parent,
            pinned: false,
        }
    }

    fn separator() -> Self {
        Self {
            prefix: String::new(),
            label: "\x00sep".into(),
            freq: String::new(),
            temp: String::new(),
            current: String::new(),
            total: String::new(),
            label_style: DIM,
            current_style: DIM,
            key: None,
            parent: None,
            pinned: false,
        }
    }

    fn has_children_in(&self, rows: &[TreeRow]) -> bool {
        self.key
            .map(|k| rows.iter().any(|r| r.parent == Some(k)))
            .unwrap_or(false)
    }
}

// ── Cumulative energy tracker ────────────────────────────────────────────────

#[derive(Default)]
struct Wh {
    cpu: f64,
    gpu: f64,
    ssd: f64,
    fans: f64,
    sys: f64,
    battery: f64,
}

// ── SMA bank ─────────────────────────────────────────────────────────────────

macro_rules! sma_fields {
    ($($f:ident),*) => {
        struct MetricsSma { $( $f: TimeSma, )* }
        impl MetricsSma {
            fn new(w: f64) -> Self { Self { $( $f: TimeSma::new(w), )* } }
            fn set_all_windows(&mut self, s: f64) { $( self.$f.set_window(s); )* }
            fn clear_all(&mut self) { $( self.$f.clear(); )* }
        }
    }
}

sma_fields!(
    sys,
    cpu,
    cpu_package,
    gpu,
    ssd,
    fans,
    battery,
    net_down,
    net_up
);

impl MetricsSma {
    fn push_metrics(&mut self, m: &Metrics) {
        self.sys.push(m.sys_power_w);
        self.cpu.push(m.cpu.power.total_w);
        self.cpu_package.push(m.cpu.power.package_w);
        self.gpu.push(m.gpu.power_w);
        self.ssd.push(m.disk_ssd_power_w);
        self.fans
            .push(m.fans.iter().map(|f| f.estimated_power_w).sum());
        self.battery.push(m.battery.drain_w as f32);
        let net_down: f32 = m.network.iter().map(|i| i.bytes_in_per_sec as f32).sum();
        let net_up: f32 = m.network.iter().map(|i| i.bytes_out_per_sec as f32).sum();
        self.net_down.push(net_down);
        self.net_up.push(net_up);
    }
}

// ── App ──────────────────────────────────────────────────────────────────────

pub struct App {
    pub metrics: Metrics,
    pub cursor: usize,
    last_tick: Option<Instant>,
    started_at: Instant,
    wh: Wh,
    sma: MetricsSma,
    pub sma_window: u32,
    pub interval_ms: u64,
    history: BTreeMap<&'static str, VecDeque<f64>>,
    pinned: Vec<&'static str>,
    collapsed: std::collections::HashSet<&'static str>,
    total_rows: usize,
    row_keys_cache: Vec<Option<&'static str>>,
    row_parents_cache: Vec<Option<&'static str>>,
    row_is_sep: Vec<bool>,
    labels: BTreeMap<&'static str, String>,
    tree_data_y: u16,
    tree_scroll: usize,
    tree_vis_h: usize,
    term_height: u16,
}

impl App {
    pub fn new() -> Self {
        Self {
            metrics: Metrics::default(),
            cursor: 2,
            last_tick: None,
            started_at: Instant::now(),
            wh: Wh::default(),
            sma: MetricsSma::new(0.0),
            sma_window: 0,
            interval_ms: 250,
            history: BTreeMap::new(),
            pinned: Vec::new(),
            collapsed: ["network", "fans"].into_iter().collect(),
            total_rows: 0,
            row_keys_cache: Vec::new(),
            row_parents_cache: Vec::new(),
            row_is_sep: Vec::new(),
            labels: BTreeMap::new(),
            tree_data_y: 0,
            tree_scroll: 0,
            tree_vis_h: 0,
            term_height: 40,
        }
    }

    fn push_history(&mut self, key: &'static str, val: f64) {
        let buf = self
            .history
            .entry(key)
            .or_insert_with(|| VecDeque::with_capacity(HISTORY_LEN + 1));
        buf.push_back(val);
        if buf.len() > HISTORY_LEN {
            buf.pop_front();
        }
    }

    pub fn update(&mut self, m: Metrics) {
        self.sma.push_metrics(&m);

        if let Some(prev) = self.last_tick {
            let dt_h = prev.elapsed().as_secs_f64() / 3600.0;
            self.wh.cpu += m.cpu.power.total_w as f64 * dt_h;
            self.wh.gpu += m.gpu.power_w as f64 * dt_h;
            self.wh.ssd += m.disk_ssd_power_w as f64 * dt_h;
            self.wh.fans += m
                .fans
                .iter()
                .map(|f| f.estimated_power_w as f64)
                .sum::<f64>()
                * dt_h;
            self.wh.sys += m.sys_power_w as f64 * dt_h;
            self.wh.battery += m.battery.drain_w * dt_h;
        }
        self.last_tick = Some(Instant::now());

        self.push_history("system", m.sys_power_w as f64);
        self.push_history("cpu", m.cpu.power.total_w as f64);
        self.push_history("gpu", m.gpu.power_w as f64);
        self.push_history("ssd", m.disk_ssd_power_w as f64);
        self.push_history(
            "fans",
            m.fans.iter().map(|f| f.estimated_power_w as f64).sum(),
        );
        self.push_history("battery", m.battery.drain_w.abs());

        // Per-core estimated power history
        let rapl_core_w: f32 = m
            .cpu
            .power
            .domains
            .iter()
            .find(|d| d.name.eq_ignore_ascii_case("core"))
            .map(|d| d.power_w)
            .unwrap_or(0.0);
        let core_budget = if rapl_core_w > 0.0 {
            rapl_core_w
        } else {
            self.sma.cpu.get() * 0.7
        };
        let total_util: f32 = m.cpu.usage_pct.iter().sum();
        let num_cores = m.cpu.usage_pct.len();
        for (ci, &usage) in m.cpu.usage_pct.iter().enumerate() {
            let est_power = if total_util > 0.0 {
                core_budget * (usage / total_util)
            } else {
                core_budget / num_cores.max(1) as f32
            };
            let core_key = proc_core_key(ci);
            self.push_history(core_key, est_power as f64);
        }

        // Per-process RSS history for memory chart
        for p in &m.top_processes {
            let key = proc_pid_rss_key(p.pid);
            self.push_history(key, p.rss_mb);
        }

        self.metrics = m;
    }

    pub fn handle_key(&mut self, key: crossterm::event::KeyEvent) -> bool {
        use crossterm::event::KeyCode;

        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => return true,
            KeyCode::Char('c')
                if key
                    .modifiers
                    .contains(crossterm::event::KeyModifiers::CONTROL) =>
            {
                return true
            }
            KeyCode::Char('k') | KeyCode::Up => self.move_cursor(-1),
            KeyCode::Char('j') | KeyCode::Down => self.move_cursor(1),
            KeyCode::Char('h') | KeyCode::Left => self.collapse_or_parent(),
            KeyCode::Right => self.expand_or_child(),
            KeyCode::Char('r') => self.reset(),
            KeyCode::Char('a') => self.cycle_sma(),
            KeyCode::Char('l') => self.cycle_latency(),
            KeyCode::Home => self.cursor = 0,
            KeyCode::PageUp => self.move_cursor(-10),
            KeyCode::PageDown => self.move_cursor(10),
            KeyCode::Char(' ') => self.toggle_pin(),
            KeyCode::Char('-') => self.collapse_all(),
            KeyCode::Char('+') | KeyCode::Char('=') => self.expand_all(),
            _ => {}
        }
        false
    }

    fn move_cursor(&mut self, delta: i32) {
        let max = self.total_rows.saturating_sub(1) as i32;
        let mut pos = (self.cursor as i32 + delta).clamp(0, max);
        let dir = if delta >= 0 { 1 } else { -1 };
        while pos >= 0 && pos <= max && self.row_is_sep.get(pos as usize).copied().unwrap_or(false)
        {
            pos += dir;
        }
        self.cursor = pos.clamp(0, max) as usize;
    }

    pub fn handle_mouse(&mut self, mouse: crossterm::event::MouseEvent) {
        use crossterm::event::{MouseButton, MouseEventKind};
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let y = mouse.row;
                if y >= self.tree_data_y {
                    let vi = (y - self.tree_data_y) as usize;
                    if vi < self.tree_vis_h {
                        let target = self.tree_scroll + vi;
                        if target < self.total_rows
                            && !self.row_is_sep.get(target).copied().unwrap_or(false)
                        {
                            if target == self.cursor {
                                if let Some(Some(key)) = self.row_keys_cache.get(self.cursor) {
                                    if self.collapsed.contains(key) {
                                        self.collapsed.remove(key);
                                    } else if self.row_parents_cache.contains(&Some(*key)) {
                                        self.collapsed.insert(*key);
                                    }
                                }
                            } else {
                                self.cursor = target;
                            }
                        }
                    }
                }
            }
            MouseEventKind::ScrollUp => {
                self.cursor = self.cursor.saturating_sub(3);
            }
            MouseEventKind::ScrollDown => {
                self.cursor = (self.cursor + 3).min(self.total_rows.saturating_sub(1));
            }
            _ => {}
        }
    }

    fn toggle_pin(&mut self) {
        if let Some(Some(key)) = self.row_keys_cache.get(self.cursor) {
            if let Some(pos) = self.pinned.iter().position(|&k| k == *key) {
                self.pinned.remove(pos);
            } else {
                self.pinned.push(*key);
            }
        }
    }

    fn collapse_or_parent(&mut self) {
        if let Some(Some(key)) = self.row_keys_cache.get(self.cursor) {
            if !self.collapsed.contains(key) {
                if self.row_parents_cache.contains(&Some(*key)) {
                    self.collapsed.insert(*key);
                    return;
                }
            }
        }
        if let Some(Some(parent)) = self.row_parents_cache.get(self.cursor) {
            if let Some(pos) = self.row_keys_cache.iter().position(|k| *k == Some(*parent)) {
                self.cursor = pos;
            }
        }
    }

    fn expand_or_child(&mut self) {
        if let Some(Some(key)) = self.row_keys_cache.get(self.cursor) {
            if self.collapsed.remove(key) {
                return;
            }
        }
    }

    fn collapse_all(&mut self) {
        for k in self.row_keys_cache.iter().flatten() {
            if self.row_parents_cache.contains(&Some(*k)) {
                self.collapsed.insert(*k);
            }
        }
    }

    fn expand_all(&mut self) {
        self.collapsed.clear();
    }

    fn reset(&mut self) {
        self.wh = Wh::default();
        self.sma.clear_all();
        self.history.clear();
    }

    fn cycle_sma(&mut self) {
        self.sma_window = match self.sma_window {
            0 => 5,
            5 => 10,
            _ => 0,
        };
        self.sma.set_all_windows(self.sma_window as f64);
    }

    fn cycle_latency(&mut self) {
        self.interval_ms = match self.interval_ms {
            250 => 500,
            500 => 1000,
            1000 => 2000,
            _ => 250,
        };
    }

    pub fn poll_interval_ms(&self) -> u64 {
        self.interval_ms
    }

    pub fn draw(&mut self, f: &mut Frame) {
        self.term_height = f.area().height;
        let all_rows = self.build_rows();

        let rows: Vec<&TreeRow> = all_rows
            .iter()
            .filter(|r| !self.is_hidden(r, &all_rows))
            .collect();

        self.total_rows = rows.len();

        let prev_key = self.row_keys_cache.get(self.cursor).copied().flatten();
        self.row_keys_cache = rows.iter().map(|r| r.key).collect();
        self.row_parents_cache = rows.iter().map(|r| r.parent).collect();
        self.row_is_sep = rows.iter().map(|r| r.label == "\x00sep").collect();

        if let Some(pk) = prev_key {
            if let Some(pos) = self.row_keys_cache.iter().position(|k| *k == Some(pk)) {
                self.cursor = pos;
            }
        }
        self.cursor = self.cursor.min(self.total_rows.saturating_sub(1));

        for r in &rows {
            if let Some(key) = r.key {
                if !r.label.is_empty() {
                    self.labels.insert(key, r.label.clone());
                }
            }
        }

        let cursor_key = self
            .row_keys_cache
            .get(self.cursor)
            .copied()
            .flatten()
            .or_else(|| self.row_parents_cache.get(self.cursor).copied().flatten());
        let chart_keys = self.chart_keys(cursor_key);
        let chart_count = if chart_keys.is_empty() {
            0
        } else if self.pinned.is_empty() {
            1
        } else {
            self.pinned.len().max(chart_keys.len())
        };
        let chart_h = chart_count as u16 * CHART_HEIGHT;

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(0),
                Constraint::Length(chart_h),
                Constraint::Length(1),
            ])
            .split(f.area());

        self.draw_tree_buf(f, chunks[0], &rows, &all_rows);
        if !chart_keys.is_empty() {
            self.draw_charts(f, chunks[1], &chart_keys);
        }
        self.draw_footer(f, chunks[2]);
    }

    fn is_hidden(&self, row: &TreeRow, all: &[TreeRow]) -> bool {
        let mut parent = row.parent;
        while let Some(p) = parent {
            if self.collapsed.contains(p) {
                return true;
            }
            parent = all.iter().find(|r| r.key == Some(p)).and_then(|r| r.parent);
        }
        false
    }

    fn chart_keys(&self, cursor_key: Option<&'static str>) -> Vec<&'static str> {
        let mut keys: Vec<&'static str> = Vec::new();
        if let Some(ck) = cursor_key {
            if !self.pinned.contains(&ck) {
                keys.push(ck);
            }
        }
        for &pk in self.pinned.iter().rev() {
            keys.push(pk);
        }
        keys
    }

    // ── Build rows ──────────────────────────────────────────────────────────

    fn build_rows(&self) -> Vec<TreeRow> {
        let m = &self.metrics;
        let w = &self.wh;
        let s = &self.sma;
        let pin = |key: &str| -> bool { self.pinned.contains(&key) };

        let temp_groups = temps_by_category(&m.temperatures);
        let temp_info = |cat: &str| -> String {
            temp_groups
                .get(cat)
                .map(|v| {
                    let avg = v.iter().sum::<f32>() / v.len() as f32;
                    let min = v.iter().copied().fold(f32::INFINITY, f32::min);
                    let max = v.iter().copied().fold(f32::NEG_INFINITY, f32::max);
                    format!("{:.0}°C ({:.0}–{:.0})", avg, min, max)
                })
                .unwrap_or_default()
        };

        let mut rows: Vec<TreeRow> = Vec::new();

        rows.push(TreeRow::separator());

        // ── Battery
        if m.battery.present && m.battery.energy_full_wh > 0.0 {
            let batt_w = s.battery.get();
            let t = m.battery.time_remaining_min;
            let (display_w, charge_status, batt_style) = if m.battery.external_connected {
                let status = if t > 0 {
                    format!("full in {}h {:02}m", t / 60, t % 60)
                } else if m.battery.status == "Charging" {
                    "charging…".into()
                } else {
                    "on power".into()
                };
                (
                    batt_w.abs(),
                    status,
                    Style::default().fg(Color::Rgb(46, 139, 87)),
                )
            } else {
                (
                    -batt_w.abs(),
                    if t > 0 {
                        format!("{}h {:02}m remaining", t / 60, t % 60)
                    } else {
                        format!("{}", m.battery.status)
                    },
                    Style::default().fg(power_color(batt_w.abs())),
                )
            };
            let health_str = if m.battery.health_pct > 0.0 && m.battery.health_pct < 100.0 {
                format!(", health {:.0}%", m.battery.health_pct)
            } else {
                String::new()
            };
            let batt_label = format!(
                "Battery {:.0}% ({}{})",
                m.battery.percent, charge_status, health_str,
            );
            rows.push(TreeRow::pw_full(
                "battery",
                None,
                "",
                &batt_label,
                display_w,
                w.battery,
                "",
                "",
                batt_style,
                pin("battery"),
            ));
        }

        rows.push(TreeRow::separator());

        // ── CPU (top-level)
        let cpu_label = format!(
            "CPU ({} cores, {:.0}%)",
            m.cpu.num_cores,
            if m.cpu.usage_pct.is_empty() {
                0.0
            } else {
                m.cpu.usage_pct.iter().sum::<f32>() / m.cpu.usage_pct.len() as f32
            }
        );
        let cpu_power = s.cpu.get();

        rows.push(TreeRow::pw_full(
            "cpu",
            None,
            "",
            &cpu_label,
            cpu_power,
            w.cpu,
            "",
            &temp_info("CPU"),
            BOLD,
            pin("cpu"),
        ));

        // RAPL sub-domains
        if m.cpu.power.available {
            let cp = "│  ";
            for (di, domain) in m.cpu.power.domains.iter().enumerate() {
                if domain.name.contains("package") {
                    continue;
                }
                let pfx = if di == m.cpu.power.domains.len() - 1 && m.cpu.usage_pct.is_empty() {
                    format!("{}└─ ", cp)
                } else {
                    format!("{}├─ ", cp)
                };
                rows.push(TreeRow::pw(
                    "rapl_domain",
                    Some("cpu"),
                    &pfx,
                    &domain.name,
                    domain.power_w,
                    0.0,
                    Style::default(),
                    false,
                ));
            }
        }

        // Per-core utilization with estimated power
        let num_cores = m.cpu.usage_pct.len();
        if num_cores > 0 {
            let rapl_core_w: f32 = m
                .cpu
                .power
                .domains
                .iter()
                .find(|d| d.name.eq_ignore_ascii_case("core"))
                .map(|d| d.power_w)
                .unwrap_or(0.0);
            let core_budget = if rapl_core_w > 0.0 {
                rapl_core_w
            } else {
                s.cpu.get() * 0.7
            };
            let total_util: f32 = m.cpu.usage_pct.iter().sum();
            let cp = "│  ";

            for (ci, &usage) in m.cpu.usage_pct.iter().enumerate() {
                let is_last_core = ci == num_cores - 1;
                let pfx = if is_last_core {
                    format!("{}└─ ", cp)
                } else {
                    format!("{}├─ ", cp)
                };

                let freq = m.cpu.freq_mhz.get(ci).copied().unwrap_or(0);
                let freq_str = if freq > 0 {
                    format!("{} MHz", freq)
                } else {
                    String::new()
                };

                let core_temp = m.cpu.core_temps.get(ci).copied().unwrap_or(0.0);
                let temp_str = if core_temp > 0.0 {
                    format!("{:.0}°C", core_temp)
                } else {
                    String::new()
                };

                let est_power = if total_util > 0.0 {
                    core_budget * (usage / total_util)
                } else {
                    core_budget / num_cores as f32
                };

                let core_key = proc_core_key(ci);
                let usage_bar_str = usage_bar(usage);

                rows.push(TreeRow::pw_full(
                    core_key,
                    Some("cpu"),
                    &pfx,
                    &format!("cpu{:<3} ({:>5.1}%) {}", ci, usage, usage_bar_str),
                    est_power,
                    0.0,
                    &freq_str,
                    &temp_str,
                    Style::default(),
                    pin(core_key),
                ));
            }
        }

        rows.push(TreeRow::separator());

        // ── GPU (top-level)
        if m.gpu.available {
            let gpu_label = if m.gpu.utilization_pct > 0 {
                format!("GPU ({}%)", m.gpu.utilization_pct)
            } else {
                format!("GPU ({})", m.gpu.name)
            };
            let gpu_temp_str = if m.gpu.temperature_c > 0.0 {
                format!("{:.0}°C", m.gpu.temperature_c)
            } else {
                temp_info("GPU")
            };
            rows.push(TreeRow::pw_full(
                "gpu",
                None,
                "",
                &gpu_label,
                s.gpu.get(),
                w.gpu,
                &fmt_freq(m.gpu.core_clock_mhz as f32),
                &gpu_temp_str,
                BOLD,
                pin("gpu"),
            ));
            let gc = "│  ";
            if !m.gpu.name.is_empty() {
                rows.push(TreeRow::info(
                    Some("gpu"),
                    &format!("{}├─ ", gc),
                    &m.gpu.name,
                    "",
                    "",
                    Style::default().fg(power_color(s.gpu.get().abs())),
                ));
            }
            if m.gpu.utilization_pct > 0 {
                rows.push(TreeRow::info(
                    Some("gpu"),
                    &format!("{}├─ ", gc),
                    &format!(
                        "Utilization ({:>3}%) {}",
                        m.gpu.utilization_pct,
                        usage_bar(m.gpu.utilization_pct as f32)
                    ),
                    "",
                    "",
                    Style::default().fg(power_color(s.gpu.get().abs())),
                ));
            }
            if m.gpu.memory_total_mb > 0 {
                rows.push(TreeRow::info(
                    Some("gpu"),
                    &format!("{}└─ ", gc),
                    &format!(
                        "VRAM ({:.1}/{:.1} GB)",
                        m.gpu.memory_used_mb as f64 / 1024.0,
                        m.gpu.memory_total_mb as f64 / 1024.0
                    ),
                    "",
                    "",
                    DATA_STYLE,
                ));
            }
        }

        rows.push(TreeRow::separator());

        // ── SSD (top-level)
        {
            let disk_name = if m.disk.disk_name.is_empty() {
                "Disk".to_string()
            } else {
                format!("Disk ({})", m.disk.disk_name)
            };
            rows.push(TreeRow::pw_est(
                "ssd",
                None,
                "",
                &disk_name,
                s.ssd.get(),
                w.ssd,
                BOLD,
                pin("ssd"),
            ));
            let sc = "│  ";
            rows.push({
                let mut r = TreeRow::info(
                    Some("ssd"),
                    &format!("{}├─ ", sc),
                    "Read",
                    &human_rate(m.disk.read_bytes_per_sec),
                    "",
                    DATA_STYLE,
                );
                r.key = Some("disk_read");
                r
            });
            rows.push({
                let mut r = TreeRow::info(
                    Some("ssd"),
                    &format!("{}└─ ", sc),
                    "Write",
                    &human_rate(m.disk.write_bytes_per_sec),
                    "",
                    DATA_STYLE,
                );
                r.key = Some("disk_write");
                r
            });
        }

        rows.push(TreeRow::separator());

        // ── Fans (top-level)
        if !m.fans.is_empty() {
            rows.push(TreeRow::pw_est(
                "fans",
                None,
                "",
                "Fans",
                s.fans.get(),
                w.fans,
                BOLD,
                pin("fans"),
            ));
            let fc = "│  ";
            for (i, fan) in m.fans.iter().enumerate() {
                let pfx = if i == m.fans.len() - 1 {
                    format!("{}└─ ", fc)
                } else {
                    format!("{}├─ ", fc)
                };
                rows.push(TreeRow::pw_est(
                    fan_key(i),
                    Some("fans"),
                    &pfx,
                    &format!("{} ({:.0} RPM)", fan.name, fan.rpm),
                    fan.estimated_power_w,
                    0.0,
                    Style::default(),
                    pin(fan_key(i)),
                ));
            }
        }

        // ── Network (top-level)
        if !m.network.is_empty() {
            let total_down: f64 = m.network.iter().map(|i| i.bytes_in_per_sec).sum();
            let total_up: f64 = m.network.iter().map(|i| i.bytes_out_per_sec).sum();
            rows.push(TreeRow::info(
                None,
                "",
                "Network",
                &format!("↓{} ↑{}", human_rate(total_down), human_rate(total_up)),
                "",
                BOLD,
            ));
            let nc = "│  ";
            for (ni, iface) in m.network.iter().enumerate() {
                let is_last = ni == m.network.len() - 1;
                let pfx = if is_last {
                    format!("{}└─ ", nc)
                } else {
                    format!("{}├─ ", nc)
                };
                let iface_type = if iface.is_wifi { "WiFi" } else { "Ethernet" };
                let link_info = if iface.link_speed_mbps >= 1000 {
                    format!("{}, {} Gbps", iface.name, iface.link_speed_mbps / 1000)
                } else if iface.link_speed_mbps > 0 {
                    format!("{}, {} Mbps", iface.name, iface.link_speed_mbps)
                } else {
                    iface.name.clone()
                };
                let ic = if is_last { "   " } else { "│  " };
                let iface_key = proc_iface_key(ni);

                rows.push(TreeRow::info(
                    Some("network"),
                    &pfx,
                    &format!("{} ({})", iface_type, link_info),
                    "",
                    "",
                    Style::default(),
                ));
                rows.push({
                    let r = TreeRow::info(
                        Some(iface_key),
                        &format!("{}├─ ", format!("{}{}", nc, ic)),
                        "↓ Download",
                        &human_rate(iface.bytes_in_per_sec),
                        "",
                        DATA_STYLE,
                    );
                    r
                });
                rows.push({
                    let r = TreeRow::info(
                        Some(iface_key),
                        &format!("{}└─ ", format!("{}{}", nc, ic)),
                        "↑ Upload",
                        &human_rate(iface.bytes_out_per_sec),
                        "",
                        DATA_STYLE,
                    );
                    r
                });
            }
        }

        // ── Software
        rows.push(TreeRow::separator());
        {
            let visible_tree_rows = rows.iter().filter(|r| !self.is_hidden(r, &rows)).count();
            let chart_slots = if self.pinned.is_empty() {
                1
            } else {
                self.pinned.len() + 1
            };
            let reserved = visible_tree_rows + 5 + chart_slots * CHART_HEIGHT as usize;
            let proc_limit = (self.term_height as usize).saturating_sub(reserved).max(10);

            let total_rss: f64 = m.top_processes.iter().map(|p| p.rss_mb).sum();
            {
                let mut r = TreeRow::info(
                    None,
                    "",
                    &format!("Software (top {} by RSS, {:.1} MB)", proc_limit, total_rss),
                    &format!("{:.1} MB", total_rss),
                    "",
                    Style::default().add_modifier(Modifier::BOLD),
                );
                r.key = Some("software");
                r.pinned = pin("software");
                rows.push(r);
            }

            if m.top_processes.is_empty() {
                rows.push(TreeRow::info(
                    Some("software"),
                    "   ",
                    "(collecting…)",
                    "",
                    "",
                    PENDING,
                ));
            }

            let self_pid = std::process::id() as i32;
            let max_rss = m
                .top_processes
                .iter()
                .map(|p| p.rss_mb)
                .fold(0.0f64, f64::max);
            for (i, p) in m.top_processes.iter().take(proc_limit).enumerate() {
                let is_last = i == m.top_processes.len().min(proc_limit) - 1;
                let pfx = if is_last { "   " } else { "│  " };
                let color = if !p.alive {
                    Color::DarkGray
                } else if p.pid == self_pid {
                    Color::Blue
                } else {
                    rss_color(p.rss_mb as f32)
                };
                let mem_str = human_bytes(p.rss_mb * 1024.0 * 1024.0);
                let bar = rss_bar(p.rss_mb as f32, max_rss as f32);
                let pid_key = proc_pid_rss_key(p.pid);
                let label = format!("{} (pid {}, {}) {}", p.name, p.pid, mem_str, bar);
                rows.push({
                    let mut r = TreeRow::info(
                        Some("software"),
                        pfx,
                        &label,
                        &if p.cpu_pct >= 1.0 {
                            format!("{:>5.1}%", p.cpu_pct)
                        } else {
                            format!("{:>5.2}%", p.cpu_pct)
                        },
                        "",
                        Style::default().fg(color),
                    );
                    r.key = Some(pid_key);
                    r.pinned = pin(pid_key);
                    r
                });
            }
        }

        rows
    }

    // ── Rendering ──────────────────────────────────────────────────────────

    fn draw_tree_buf(
        &mut self,
        f: &mut Frame,
        area: Rect,
        rows: &[&TreeRow],
        all_rows: &[TreeRow],
    ) {
        let block = Block::default().borders(Borders::ALL).title(format!(
            " Power Tree ({}/{}) ",
            self.cursor + 1,
            rows.len()
        ));
        let inner = block.inner(area);
        f.render_widget(block, area);

        if inner.width < 20 || inner.height < 3 {
            return;
        }
        let buf = f.buffer_mut();

        let hdr_y = inner.y;
        let right = inner.right();
        let spark_gap: u16 = if inner.width > 90 { 1 } else { 0 };
        let spark_w = if inner.width > 90 {
            (inner.width - 90 - 1).min(60)
        } else {
            0
        };
        let tot_x = right
            .saturating_sub(COL_TOT)
            .saturating_sub(spark_w)
            .saturating_sub(spark_gap);
        let cur_x = tot_x.saturating_sub(COL_CUR);
        let tmp_x = cur_x.saturating_sub(COL_TEMP);
        let frq_x = tmp_x.saturating_sub(COL_FREQ);
        let spark_x = right.saturating_sub(spark_w);

        buf.set_string(inner.x + 2, hdr_y, "Component", BOLD);
        right_str(buf, frq_x, hdr_y, COL_FREQ, "Freq", BOLD);
        right_str(buf, tmp_x, hdr_y, COL_TEMP, "Temp", BOLD);
        right_str(buf, cur_x, hdr_y, COL_CUR, "Power", BOLD);
        right_str(buf, tot_x, hdr_y, COL_TOT, "Cumulative", BOLD);
        if spark_w > 0 {
            right_str(buf, spark_x, hdr_y, spark_w, "History", BOLD);
        }

        let data_y = hdr_y + 1;
        let vis_h = inner.height.saturating_sub(1) as usize;
        let total = rows.len();
        let scroll = self.scroll_offset(vis_h, total);
        self.tree_data_y = data_y;
        self.tree_scroll = scroll;
        self.tree_vis_h = vis_h;
        let pin_w: u16 = 2;
        let tree_x = inner.x + pin_w;

        for (vi, row) in rows.iter().skip(scroll).take(vis_h).enumerate() {
            let y = data_y + vi as u16;
            let abs_idx = scroll + vi;

            if row.label == "\x00sep" {
                let line = "─".repeat(inner.width as usize);
                buf.set_string(inner.x, y, &line, TREE_STYLE);
                continue;
            }

            if row.pinned {
                buf.set_string(inner.x, y, PIN_MARKER, Style::default().fg(Color::Cyan));
            }

            buf.set_string(tree_x, y, &row.prefix, TREE_STYLE);

            let label_x = tree_x + row.prefix.width() as u16;
            let is_parent = row.has_children_in(all_rows);
            let is_collapsed = row.key.map(|k| self.collapsed.contains(k)).unwrap_or(false);
            let indicator = if is_parent {
                if is_collapsed {
                    "▸ "
                } else {
                    "▾ "
                }
            } else {
                ""
            };
            let max_label_w = cur_x.saturating_sub(label_x) as usize;
            if is_parent {
                buf.set_string(label_x, y, indicator, TREE_STYLE);
                let lbl_start = label_x + indicator.width() as u16;
                let lbl_text =
                    truncate_str(&row.label, max_label_w.saturating_sub(indicator.width()));
                buf.set_string(lbl_start, y, &lbl_text, row.label_style);
            } else {
                let full_label = format!("{}{}", indicator, row.label);
                let truncated_label = truncate_str(&full_label, max_label_w);
                buf.set_string(label_x, y, &truncated_label, row.label_style);
            }

            if !row.freq.is_empty() {
                buf.set_string(frq_x, y, " ".repeat(COL_FREQ as usize), Style::default());
                right_str(buf, frq_x, y, COL_FREQ, &row.freq, DIM);
            }
            if !row.temp.is_empty() {
                buf.set_string(tmp_x, y, " ".repeat(COL_TEMP as usize), Style::default());
                right_str(buf, tmp_x, y, COL_TEMP, &row.temp, DIM);
            }
            if !row.current.is_empty() {
                right_str(buf, cur_x, y, COL_CUR, &row.current, row.current_style);
            }
            if !row.total.is_empty() {
                right_str(buf, tot_x, y, COL_TOT, &row.total, DIM);
            }

            if spark_w > 0 {
                if let Some(key) = row.key {
                    if let Some(hist) = self.history.get(key) {
                        let is_mem = key.starts_with("prss_");
                        let pick_color = |v: f32| {
                            if is_mem {
                                rss_color(v)
                            } else {
                                power_color(v)
                            }
                        };
                        let w = spark_w as usize;
                        let skip = hist.len().saturating_sub(w);
                        let visible: Vec<f64> = hist.iter().skip(skip).copied().collect();
                        let vis_max = visible.iter().copied().fold(0.0f64, f64::max).max(1e-6);
                        for (ci, &val) in visible.iter().enumerate() {
                            let x = spark_x + (w - visible.len() + ci) as u16;
                            let level = (val / vis_max * 7.0).round() as usize;
                            let ch = SPARK_CHARS[level.min(7)];
                            buf.set_string(
                                x,
                                y,
                                ch.to_string(),
                                Style::default().fg(pick_color(val as f32)),
                            );
                        }
                    }
                }
            }

            if abs_idx == self.cursor {
                for cx in inner.x..inner.right() {
                    if let Some(cell) = buf.cell_mut(ratatui::layout::Position::new(cx, y)) {
                        cell.set_bg(Color::Rgb(50, 50, 60));
                    }
                }
            }
        }
    }

    fn scroll_offset(&self, vis_h: usize, total: usize) -> usize {
        if self.cursor < vis_h / 3 {
            0
        } else if self.cursor > total.saturating_sub(vis_h * 2 / 3) {
            total.saturating_sub(vis_h)
        } else {
            self.cursor.saturating_sub(vis_h / 3)
        }
    }

    fn draw_charts(&self, f: &mut Frame, area: Rect, keys: &[&'static str]) {
        if keys.is_empty() || area.height == 0 {
            return;
        }

        let constraints: Vec<Constraint> = keys
            .iter()
            .map(|_| Constraint::Length(CHART_HEIGHT))
            .collect();
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(area);

        for (i, &key) in keys.iter().enumerate() {
            if i >= chunks.len() {
                break;
            }
            let is_mem_key = key.starts_with("prss_");
            let data = self.history.get(key);
            let current = data.and_then(|b| b.back().copied()).unwrap_or(0.0);

            let is_pinned = self.pinned.contains(&key);
            let title_style = if is_pinned {
                Style::default().fg(Color::Cyan)
            } else {
                Style::default().fg(Color::Reset)
            };
            let pin_icon = if is_pinned { " [pinned]" } else { "" };

            let chart_area = chunks[i];
            let inner = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Length(6), Constraint::Min(0)])
                .split(chart_area);

            let vis_w = inner[1].width.saturating_sub(2) as usize;
            let visible_data: Vec<f64> = data
                .map(|b| {
                    let skip = b.len().saturating_sub(vis_w);
                    b.iter().skip(skip).copied().collect()
                })
                .unwrap_or_default();
            let vis_max = visible_data.iter().copied().fold(0.0f64, f64::max);
            let scale_max = if is_mem_key {
                nice_mem_scale(vis_max)
            } else {
                nice_scale(vis_max)
            };

            let scale_h = inner[0].height;
            let fmt_axis = if is_mem_key {
                |v: f64| -> String { fmt_mem_axis(v) }
            } else {
                |v: f64| -> String {
                    if v.abs() >= 1.0 {
                        format!("{:.1}W", v)
                    } else {
                        let mw = v * 1000.0;
                        if mw.abs() >= 1.0 {
                            format!("{:.0}mW", mw)
                        } else {
                            format!("{:.1}mW", mw)
                        }
                    }
                }
            };
            let scale_lines: Vec<Line> = (0..scale_h)
                .map(|row| {
                    if row == 0 {
                        Line::from(Span::styled(fmt_axis(scale_max), DIM))
                    } else if row == scale_h / 2 {
                        Line::from(Span::styled(fmt_axis(scale_max / 2.0), DIM))
                    } else if row == scale_h - 1 {
                        Line::from(Span::styled(fmt_axis(0.0), DIM))
                    } else {
                        Line::from("")
                    }
                })
                .collect();
            f.render_widget(Paragraph::new(scale_lines), inner[0]);

            let title_value = if is_mem_key {
                fmt_mem_value(current)
            } else if current.abs() >= 1.0 {
                format!("{:.3} W", current)
            } else {
                format!("{:.1} mW", current * 1000.0)
            };
            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(DIM)
                .title(Span::styled(
                    format!(
                        " {} — {}{}",
                        self.labels.get(key).map(|s| s.as_str()).unwrap_or(key),
                        title_value,
                        pin_icon
                    ),
                    title_style,
                ));
            let chart_inner = block.inner(inner[1]);
            f.render_widget(block, inner[1]);

            if chart_inner.width == 0 || chart_inner.height == 0 || scale_max <= 0.0 {
                continue;
            }

            let buf = f.buffer_mut();
            let inner_h = chart_inner.height as usize;
            let max_eighths = inner_h * 8;
            let bottom_y = chart_inner.y + chart_inner.height - 1;

            for (ci, &val) in visible_data.iter().enumerate() {
                let x = chart_inner.x + (vis_w.saturating_sub(visible_data.len()) + ci) as u16;
                if x >= chart_inner.right() {
                    continue;
                }
                let color = if is_mem_key {
                    rss_color(val as f32)
                } else {
                    power_color(val as f32)
                };
                let bar_eighths =
                    ((val / scale_max * max_eighths as f64).round() as usize).min(max_eighths);
                let full_rows = bar_eighths / 8;
                let remainder = bar_eighths % 8;
                let style = Style::default().fg(color);

                for row in 0..full_rows {
                    let y = bottom_y.saturating_sub(row as u16);
                    if y >= chart_inner.y {
                        buf.set_string(x, y, "█", style);
                    }
                }
                if remainder > 0 {
                    let y = bottom_y.saturating_sub(full_rows as u16);
                    if y >= chart_inner.y {
                        buf.set_string(x, y, BAR_EIGHTHS[remainder].to_string(), style);
                    }
                }
            }
        }
    }

    fn draw_footer(&self, f: &mut Frame, area: Rect) {
        let cursor_key = self.row_keys_cache.get(self.cursor).copied().flatten();
        let is_mem = cursor_key.map(|k| k.starts_with("prss_")).unwrap_or(false);

        let (l1, l2, l3, l4) = if is_mem {
            ("<10M", "<100M", "<1G", "≥1G")
        } else {
            ("<5W", "<15W", "<30W", "≥30W")
        };

        let elapsed = self.started_at.elapsed();
        let secs = elapsed.as_secs();
        let uptime = if secs >= 3600 {
            format!(
                "{}h {:02}m {:02}s",
                secs / 3600,
                (secs % 3600) / 60,
                secs % 60
            )
        } else {
            format!("{:02}m {:02}s", (secs % 3600) / 60, secs % 60)
        };

        let spans: Vec<Span> = vec![
            Span::styled(" q", Style::default().fg(Color::Yellow)),
            Span::raw(" quit  "),
            Span::styled("r", Style::default().fg(Color::Yellow)),
            Span::raw(" reset  "),
            Span::styled("a", Style::default().fg(Color::Yellow)),
            Span::raw(format!(" avg:{}s  ", self.sma_window)),
            Span::styled("l", Style::default().fg(Color::Yellow)),
            Span::raw(format!(" {}ms  ", self.interval_ms)),
            Span::styled("↑↓←→+-", Style::default().fg(Color::Yellow)),
            Span::raw(" tree  "),
            Span::styled("space", Style::default().fg(Color::Yellow)),
            Span::raw(" pin    "),
            Span::styled("■", Style::default().fg(Color::Rgb(46, 139, 87))),
            Span::raw(format!("{} ", l1)),
            Span::styled("■", Style::default().fg(Color::Rgb(220, 180, 0))),
            Span::raw(format!("{} ", l2)),
            Span::styled("■", Style::default().fg(Color::Rgb(255, 140, 0))),
            Span::raw(format!("{} ", l3)),
            Span::styled("■", Style::default().fg(Color::Rgb(255, 50, 50))),
            Span::raw(l4),
        ];

        let buf = f.buffer_mut();
        let mut x = area.x;
        let max_x = area.x + area.width;
        for span in &spans {
            if x >= max_x {
                break;
            }
            let remaining_w = (max_x - x) as usize;
            let content = span.content.as_ref();
            let truncated = truncate_str(content, remaining_w);
            buf.set_string(x, area.y, &truncated, span.style);
            x += truncated.width() as u16;
        }

        // Draw uptime right-aligned
        let uptime_label = format!(" ⏱ {} ", uptime);
        let ux = area.x + area.width.saturating_sub(uptime_label.width() as u16);
        buf.set_string(ux, area.y, &uptime_label, DIM);
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn right_str(buf: &mut Buffer, x: u16, y: u16, width: u16, text: &str, style: Style) {
    let tw = text.width() as u16;
    let start = if tw >= width { x } else { x + width - tw };
    buf.set_string(start, y, text, style);
}

fn truncate_str(s: &str, max_w: usize) -> String {
    if s.width() <= max_w {
        return s.to_string();
    }
    let mut w = 0;
    let mut end = 0;
    for (i, ch) in s.char_indices() {
        let cw = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if w + cw > max_w.saturating_sub(1) {
            break;
        }
        w += cw;
        end = i + ch.len_utf8();
    }
    format!("{}…", &s[..end])
}

fn temps_by_category(temps: &[TempSensor]) -> BTreeMap<String, Vec<f32>> {
    temps.iter().filter(|t| t.category != "Other").fold(
        BTreeMap::<String, Vec<f32>>::new(),
        |mut m: BTreeMap<String, Vec<f32>>, t: &TempSensor| {
            m.entry(t.category.clone())
                .or_default()
                .push(t.value_celsius);
            m
        },
    )
}

fn nice_scale(max_val: f64) -> f64 {
    if max_val <= 0.0 {
        return 1.0;
    }
    let steps = [0.1, 0.2, 0.5, 1.0, 2.0, 5.0, 10.0, 20.0, 50.0, 100.0, 200.0];
    steps
        .iter()
        .copied()
        .find(|&s| s >= max_val)
        .unwrap_or(max_val.ceil().max(1.0))
}

/// Memory scale steps: 10 MiB, 100 MiB, 1 GiB, 5 GiB, ...
fn nice_mem_scale(max_val: f64) -> f64 {
    if max_val <= 0.0 {
        return 10.0;
    }
    let steps = [10.0, 100.0, 250.0, 500.0, 1024.0, 2048.0, 5120.0, 10240.0];
    steps
        .iter()
        .copied()
        .find(|&s| s >= max_val)
        .unwrap_or(max_val.ceil().max(10.0))
}

fn fmt_mem_axis(mb: f64) -> String {
    if mb <= 0.0 {
        return "0".into();
    }
    if mb < 1024.0 {
        if mb < 100.0 {
            format!("{:.0}M", mb)
        } else {
            format!("{:.0}M", mb)
        }
    } else {
        let gb = mb / 1024.0;
        if gb < 10.0 {
            format!("{:.1}G", gb)
        } else {
            format!("{:.0}G", gb)
        }
    }
}

fn fmt_mem_value(mb: f64) -> String {
    if mb <= 0.0 {
        return "0 B".into();
    }
    if mb < 1024.0 {
        format!("{:.1} MiB", mb)
    } else {
        format!("{:.2} GiB", mb / 1024.0)
    }
}

fn fan_key(index: usize) -> &'static str {
    const KEYS: [&str; 8] = [
        "fan0", "fan1", "fan2", "fan3", "fan4", "fan5", "fan6", "fan7",
    ];
    KEYS.get(index).copied().unwrap_or("fan0")
}

fn proc_iface_key(index: usize) -> &'static str {
    const KEYS: [&str; 8] = [
        "iface0", "iface1", "iface2", "iface3", "iface4", "iface5", "iface6", "iface7",
    ];
    KEYS.get(index).copied().unwrap_or("iface0")
}

fn proc_core_key(index: usize) -> &'static str {
    const KEYS: [&str; 32] = [
        "core0", "core1", "core2", "core3", "core4", "core5", "core6", "core7", "core8", "core9",
        "core10", "core11", "core12", "core13", "core14", "core15", "core16", "core17", "core18",
        "core19", "core20", "core21", "core22", "core23", "core24", "core25", "core26", "core27",
        "core28", "core29", "core30", "core31",
    ];
    KEYS.get(index).copied().unwrap_or("core0")
}

/// Box::leak creates a unique static str per PID for history tracking.
/// Safe: PIDs are reused slowly enough that history bloat is bounded.
fn proc_pid_rss_key(pid: i32) -> &'static str {
    Box::leak(format!("prss_{}", pid).into_boxed_str())
}

/// Color scale based on RSS memory usage (MB)
fn rss_color(rss_mb: f32) -> Color {
    match rss_mb {
        r if r < 10.0 => Color::Rgb(46, 139, 87), // green: < 10 MiB
        r if r < 100.0 => Color::Rgb(220, 180, 0), // yellow: < 100 MiB
        r if r < 1024.0 => Color::Rgb(255, 140, 0), // orange: < 1 GiB
        _ => Color::Rgb(255, 50, 50),             // red: >= 1 GiB
    }
}

/// RSS bar: 10-char mini bar, width proportional to max RSS in the list
fn rss_bar(rss_mb: f32, max_rss: f32) -> String {
    let width = 10;
    if max_rss <= 0.0 {
        return "░░░░░░░░░░".to_string();
    }
    let filled = ((rss_mb / max_rss) * width as f32).round() as usize;
    let filled = filled.min(width);
    format!("{}{}", "█".repeat(filled), "░".repeat(width - filled))
}
