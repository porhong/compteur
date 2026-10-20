use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use eframe::egui::{
    self, Color32, CornerRadius, FontId, Galley, Id, PointerButton, Pos2, Rect, Sense, Vec2, ViewportBuilder,
    ViewportCommand, ViewportId,
    text::{LayoutJob, TextFormat},
};

use crate::{
    config::{Config, Metric},
    format::{self, Level},
    sampler::{self, HISTORY_LEN, History, Settings, Shared},
};

pub const STRIP_HEIGHT: f32 = 24.0;
const PADDING_X: f32 = 10.0;
const FONT_SIZE: f32 = 13.0;
const DETAIL_FONT_SIZE: f32 = 12.0;
const DETAIL_PADDING: f32 = 8.0;
const POPUP_GAP: f32 = 4.0;
const CORNER_RADIUS: u8 = 6;
const SEPARATOR: &str = " │ ";
const LABEL_COLOR: Color32 = Color32::from_gray(150);
const TEXT_COLOR: Color32 = Color32::from_gray(230);
const WARN_COLOR: Color32 = Color32::from_rgb(255, 190, 70);
const HOT_COLOR: Color32 = Color32::from_rgb(255, 95, 85);
/// Wait for the window to stop moving (or a slider to settle) before writing the config to disk.
const SAVE_DELAY: Duration = Duration::from_millis(800);
/// Used to place the menu before its real size is known; it then resizes itself to fit.
const MENU_SIZE: Vec2 = Vec2::new(230.0, 330.0);
const REFRESH_CHOICES: [(u64, &str); 3] = [(500, "0.5 s"), (1000, "1 s"), (2000, "2 s")];

pub struct App {
    config: Config,
    /// Last state written to disk; differs from `config` while a save is pending.
    saved: Config,
    dirty_since: Option<Instant>,
    shared: Arc<Mutex<Shared>>,
    menu: Option<MenuState>,
}

struct MenuState {
    position: Pos2,
    /// The menu closes on focus loss, but only after it has had focus once.
    was_focused: bool,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, config: Config) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        let shared = sampler::spawn(cc.egui_ctx.clone(), Settings::from_config(&config));
        Self { saved: config.clone(), config, dirty_since: None, shared, menu: None }
    }

    fn save_now(&mut self) {
        if let Err(err) = self.config.save() {
            eprintln!("compteur: failed to save config: {err}");
        }
        self.saved = self.config.clone();
        self.dirty_since = None;
    }

    /// Records the window position and saves any config change once it has been stable for `SAVE_DELAY`.
    fn persist(&mut self, ctx: &egui::Context) {
        let position = ctx.input(|i| i.viewport().outer_rect).map(|r| [r.min.x.round(), r.min.y.round()]);
        if position.is_some() && position != self.config.position {
            self.config.position = position;
            self.dirty_since = Some(Instant::now()); // restart the wait while the strip keeps moving
        }
        if self.config == self.saved {
            self.dirty_since = None;
            return;
        }
        let since = *self.dirty_since.get_or_insert_with(Instant::now);
        if since.elapsed() >= SAVE_DELAY {
            self.save_now();
        } else {
            ctx.request_repaint_after(SAVE_DELAY - since.elapsed());
        }
    }

    /// Shrinks or grows the window to fit the strip's content.
    fn fit_window(ctx: &egui::Context, size: Vec2) {
        let current = ctx.input(|i| i.viewport().inner_rect).map(|r| r.size());
        if current.is_none_or(|current| (current - size).length() > 0.5) {
            ctx.send_viewport_cmd(ViewportCommand::InnerSize(size));
        }
    }

    fn show_menu(&mut self, ctx: &egui::Context) {
        let Some(menu) = &mut self.menu else { return };
        let mut close = false;
        let mut exit = false;

        let config = &mut self.config;
        let builder = popup_builder("Compteur menu", menu.position, MENU_SIZE);
        ctx.show_viewport_immediate(ViewportId::from_hash_of("menu"), builder, |ui, _class| {
            let focused = ui.ctx().input(|i| i.viewport().focused);
            menu.was_focused |= focused == Some(true);
            if (menu.was_focused && focused == Some(false))
                || ui.input(|i| i.key_pressed(egui::Key::Escape) || i.viewport().close_requested())
            {
                close = true;
            }
            let panel = egui::CentralPanel::default().show(ui, |ui| {
                exit = menu_contents(ui, config);
                ui.min_rect().size()
            });
            // Fit the window to its contents plus the panel's 8 pt margin on each side.
            let wanted = (panel.inner + Vec2::splat(16.0)).ceil();
            let current = ui.ctx().input(|i| i.viewport().inner_rect).map(|r| r.size());
            if current.is_some_and(|current| (current - wanted).length() > 1.0) {
                ui.ctx().send_viewport_cmd(ViewportCommand::InnerSize(wanted));
            }
        });

        if exit {
            self.save_now();
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
        if close || exit {
            self.menu = None;
        }
    }
}

impl eframe::App for App {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0; 4] // fully transparent, so only the rounded strip is visible
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.persist(&ctx);

        let mut shared = sampler::lock(&self.shared);
        shared.settings = Settings::from_config(&self.config);
        let segments: Vec<(Metric, Arc<Galley>)> = segments(&shared)
            .into_iter()
            .filter(|(metric, _)| self.config.shown.get(*metric))
            .map(|(metric, job)| (metric, ui.painter().layout_job(job)))
            .collect();
        let separator = ui.painter().layout_no_wrap(SEPARATOR.to_owned(), FontId::monospace(FONT_SIZE), LABEL_COLOR);

        let content_width = segments.iter().map(|(_, galley)| galley.size().x).sum::<f32>()
            + separator.size().x * segments.len().saturating_sub(1) as f32;
        let size = Vec2::new((content_width + 2.0 * PADDING_X).ceil(), STRIP_HEIGHT);
        Self::fit_window(&ctx, size);

        let rect = Rect::from_min_size(ui.max_rect().min, size);
        let background = Color32::from_black_alpha((self.config.opacity * 255.0) as u8);
        ui.painter().rect_filled(rect, CornerRadius::same(CORNER_RADIUS), background);

        // Lay segments out left to right, remembering each one's x-range for hover lookup.
        let mut x = rect.left() + PADDING_X;
        let mut spans = Vec::with_capacity(segments.len());
        for (index, (metric, galley)) in segments.into_iter().enumerate() {
            if index > 0 {
                let y = rect.center().y - separator.size().y / 2.0;
                ui.painter().galley(Pos2::new(x, y), separator.clone(), LABEL_COLOR);
                x += separator.size().x;
            }
            let width = galley.size().x;
            ui.painter().galley(Pos2::new(x, rect.center().y - galley.size().y / 2.0), galley, TEXT_COLOR);
            spans.push((metric, x..x + width));
            x += width;
        }

        let response = ui.interact(rect, Id::unique("strip"), Sense::click_and_drag());
        if response.drag_started_by(PointerButton::Primary) && !self.config.locked {
            ctx.send_viewport_cmd(ViewportCommand::StartDrag);
        }
        let strip_origin = ctx.input(|i| i.viewport().outer_rect).map_or(Pos2::ZERO, |r| r.min);
        let monitor_height = ctx.input(|i| i.viewport().monitor_size).map(|s| s.y);
        if response.secondary_clicked() {
            let click_x = response.interact_pointer_pos().map_or(0.0, |p| p.x);
            let y = popup_y(strip_origin.y, MENU_SIZE.y, monitor_height);
            self.menu = Some(MenuState { position: Pos2::new(strip_origin.x + click_x, y), was_focused: false });
        }

        // Details for the hovered segment, unless the menu or a drag is in progress.
        let hovered = response
            .hover_pos()
            .filter(|_| self.menu.is_none() && !response.dragged())
            .and_then(|pos| spans.iter().find(|(_, span)| span.contains(&pos.x)))
            .map(|(metric, span)| (*metric, span.start));
        let details = hovered.map(|(metric, x)| (x, detail_lines(metric, &shared, self.config.refresh_ms)));
        drop(shared);

        if let Some((x, lines)) = details {
            let galleys: Vec<_> = lines
                .into_iter()
                .map(|line| ui.painter().layout_no_wrap(line, FontId::monospace(DETAIL_FONT_SIZE), TEXT_COLOR))
                .collect();
            show_details(&ctx, strip_origin, x - PADDING_X, monitor_height, galleys);
        }
        self.show_menu(&ctx);
    }
}

/// Settings menu. Changes apply immediately; returns true when Exit was clicked.
fn menu_contents(ui: &mut egui::Ui, config: &mut Config) -> bool {
    ui.label(egui::RichText::new("Show on strip").strong());
    let shown_count = config.shown.count();
    for metric in Metric::ALL {
        let mut on = config.shown.get(metric);
        // Keep at least one metric, or the strip would vanish along with its menu.
        let is_last = on && shown_count == 1;
        if ui.add_enabled(!is_last, egui::Checkbox::new(&mut on, metric.label())).changed() {
            config.shown.set(metric, on);
        }
    }

    ui.separator();
    ui.horizontal(|ui| {
        ui.label("Refresh");
        for (ms, label) in REFRESH_CHOICES {
            ui.selectable_value(&mut config.refresh_ms, ms, label);
        }
    });
    ui.horizontal(|ui| {
        ui.label("Opacity");
        ui.add(egui::Slider::new(&mut config.opacity, 0.2..=1.0).custom_formatter(|v, _| format!("{:.0}%", v * 100.0)));
    });
    ui.checkbox(&mut config.locked, "Lock position");

    ui.separator();
    ui.button("Exit Compteur").clicked()
}

fn level_color(level: Level) -> Color32 {
    match level {
        Level::Normal => TEXT_COLOR,
        Level::Warn => WARN_COLOR,
        Level::Hot => HOT_COLOR,
    }
}

/// One strip segment built from differently colored text runs.
fn segment_job(parts: &[(&str, Color32)]) -> LayoutJob {
    let font = FontId::monospace(FONT_SIZE);
    let mut job = LayoutJob::default();
    for (text, color) in parts {
        job.append(text, 0.0, TextFormat::simple(font.clone(), *color));
    }
    job
}

fn percent_color(percent: Option<f32>) -> Color32 {
    level_color(percent.map_or(Level::Normal, Level::for_percent))
}

fn percent_segment(label: &str, percent: Option<f32>) -> LayoutJob {
    segment_job(&[(label, LABEL_COLOR), (&format::percent(percent), percent_color(percent))])
}

fn segments(shared: &Shared) -> Vec<(Metric, LayoutJob)> {
    let snapshot = &shared.snapshot;
    let network = snapshot.network.as_ref();
    let disk = snapshot.disk.as_ref();
    let gpu = snapshot.gpu.as_ref();
    // Disk rates are colored by how busy the disks are, not by throughput.
    let disk_color = percent_color(disk.and_then(|d| d.active_percent));
    vec![
        (Metric::Cpu, percent_segment("CPU ", snapshot.cpu.as_ref().map(|cpu| cpu.usage))),
        (Metric::Memory, percent_segment("RAM ", snapshot.memory.as_ref().map(|memory| memory.percent()))),
        (Metric::Gpu, percent_segment("GPU ", gpu.and_then(|g| g.usage()))),
        (Metric::Vram, percent_segment("VRAM ", gpu.and_then(|g| g.main_adapter()?.vram_percent()))),
        (
            Metric::Network,
            segment_job(&[
                ("NET ↓", LABEL_COLOR),
                (&format::rate(network.map(|n| n.down_per_sec)), TEXT_COLOR),
                (" ↑", LABEL_COLOR),
                (&format::rate(network.map(|n| n.up_per_sec)), TEXT_COLOR),
            ]),
        ),
        (
            Metric::Disk,
            segment_job(&[
                ("DSK R", LABEL_COLOR),
                (&format::rate(disk.and_then(|d| d.read_per_sec)), disk_color),
                (" W", LABEL_COLOR),
                (&format::rate(disk.and_then(|d| d.write_per_sec)), disk_color),
            ]),
        ),
    ]
}

fn peak(history: &History) -> f32 {
    history.iter().fold(0.0, f32::max)
}

fn detail_lines(metric: Metric, shared: &Shared, refresh_ms: u64) -> Vec<String> {
    let window_secs = HISTORY_LEN as u64 * refresh_ms / 1000;
    let snapshot = &shared.snapshot;
    match metric {
        Metric::Cpu => {
            let Some(cpu) = &snapshot.cpu else { return vec!["CPU: measuring…".to_owned()] };
            let mut lines = vec![
                format!("CPU {:.0}% at {:.2} GHz", cpu.usage, cpu.frequency_mhz as f32 / 1000.0),
                format!("Peak, last {window_secs}s: {:.0}%", peak(&shared.cpu_history)),
                format!("{} logical cores (%):", cpu.per_core.len()),
            ];
            lines.extend(
                cpu.per_core.chunks(8).map(|row| row.iter().map(|u| format!("{u:>3.0}")).collect::<Vec<_>>().join(" ")),
            );
            lines
        }
        Metric::Memory => {
            let Some(memory) = &snapshot.memory else { return vec!["RAM: measuring…".to_owned()] };
            vec![
                format!(
                    "RAM {} / {} ({:.0}%)",
                    format::bytes(memory.used),
                    format::bytes(memory.total),
                    memory.percent()
                ),
                format!("Available: {}", format::bytes(memory.total.saturating_sub(memory.used))),
                format!("Swap: {} / {}", format::bytes(memory.swap_used), format::bytes(memory.swap_total)),
                format!("Peak, last {window_secs}s: {:.0}%", peak(&shared.memory_history)),
            ]
        }
        Metric::Gpu | Metric::Vram => {
            let Some(gpu) = &snapshot.gpu else { return vec!["GPU: no hardware GPU counters".to_owned()] };
            let mut lines = Vec::new();
            for adapter in &gpu.adapters {
                lines.push(adapter.name.clone());
                let busy: Vec<String> = adapter
                    .engines
                    .iter()
                    .filter(|(_, load)| *load >= 0.5)
                    .take(3)
                    .map(|(kind, load)| format!("{kind} {load:.0}%"))
                    .collect();
                let breakdown = if busy.is_empty() { String::new() } else { format!("  ({})", busy.join(", ")) };
                lines.push(format!("  Load {:.0}%{breakdown}", adapter.usage));
                let percent = adapter.vram_percent().map_or(String::new(), |p| format!(" ({p:.0}%)"));
                lines.push(format!(
                    "  VRAM {} / {}{percent}",
                    format::bytes(adapter.vram_used),
                    format::bytes(adapter.vram_total)
                ));
            }
            lines.push(format!("Peak load, last {window_secs}s: {:.0}%", peak(&shared.gpu_history)));
            lines
        }
        Metric::Network => {
            let Some(network) = &snapshot.network else { return vec!["Network: measuring…".to_owned()] };
            let mut lines = vec![format!(
                "Down {}/s   Up {}/s",
                format::bytes(network.down_per_sec as u64),
                format::bytes(network.up_per_sec as u64)
            )];
            lines.extend(network.adapters.iter().map(|a| {
                format!(
                    "  {}: ↓{}/s ↑{}/s",
                    a.name,
                    format::bytes(a.down_per_sec as u64),
                    format::bytes(a.up_per_sec as u64)
                )
            }));
            if network.adapters.is_empty() {
                lines.push("  No active network adapters".to_owned());
            }
            lines
        }
        Metric::Disk => {
            let Some(disk) = &snapshot.disk else { return vec!["Disk: measuring…".to_owned()] };
            let mut lines = vec![
                format!("Disk active: {}", format::percent(disk.active_percent).trim()),
                format!(
                    "Read {}/s   Write {}/s",
                    format::rate(disk.read_per_sec).trim(),
                    format::rate(disk.write_per_sec).trim()
                ),
                "Free space:".to_owned(),
            ];
            lines.extend(disk.volumes.iter().map(|v| {
                let percent = if v.total == 0 { 0.0 } else { v.used as f64 / v.total as f64 * 100.0 };
                format!(
                    "  {}  {} free of {} ({percent:.0}% used)",
                    v.mount,
                    format::bytes(v.total - v.used),
                    format::bytes(v.total)
                )
            }));
            lines
        }
    }
}

/// The strip is too small to host egui popups, so menus and details are their own borderless windows.
fn popup_builder(title: &str, position: Pos2, size: Vec2) -> ViewportBuilder {
    ViewportBuilder::default()
        .with_title(title)
        .with_position(position)
        .with_inner_size(size)
        .with_decorations(false)
        .with_resizable(false)
        .with_window_level(egui::WindowLevel::AlwaysOnTop)
        .with_taskbar(false)
}

/// Top of a popup `height` tall: below the strip, or above it when it would run off the screen.
fn popup_y(strip_top: f32, height: f32, monitor_height: Option<f32>) -> f32 {
    let below = strip_top + STRIP_HEIGHT + POPUP_GAP;
    if monitor_height.is_none_or(|h| below + height <= h) { below } else { strip_top - POPUP_GAP - height }
}

/// Shows `lines` below the strip (or above it, near the bottom of the screen), aligned to `x`.
fn show_details(ctx: &egui::Context, strip_origin: Pos2, x: f32, monitor_height: Option<f32>, lines: Vec<Arc<Galley>>) {
    let width = lines.iter().map(|g| g.size().x).fold(0.0, f32::max);
    let height: f32 = lines.iter().map(|g| g.size().y).sum();
    let size = Vec2::new(width, height) + Vec2::splat(2.0 * DETAIL_PADDING);

    let y = popup_y(strip_origin.y, size.y, monitor_height);
    let builder = popup_builder("Compteur details", Pos2::new(strip_origin.x + x, y), size.ceil())
        .with_active(false)
        .with_mouse_passthrough(true);

    ctx.show_viewport_immediate(ViewportId::from_hash_of("details"), builder, |ui, _class| {
        let rect = ui.max_rect();
        ui.painter().rect_filled(rect, CornerRadius::ZERO, Color32::from_gray(24));
        let mut pos = rect.min + Vec2::splat(DETAIL_PADDING);
        for galley in lines.clone() {
            let height = galley.size().y;
            ui.painter().galley(pos, galley, TEXT_COLOR);
            pos.y += height;
        }
    });
}
