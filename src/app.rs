use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use eframe::egui::{
    self, Color32, CornerRadius, FontId, Galley, Id, PointerButton, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2,
    ViewportBuilder, ViewportCommand, ViewportId,
    text::{LayoutJob, TextFormat},
};

use crate::{
    config::{Config, Metric, Shown},
    controls::{Action, Controls},
    platform::{self, Window},
    format::{self, Level},
    sampler::{self, HISTORY_LEN, History, Settings, Shared},
};

pub const STRIP_HEIGHT: f32 = 24.0;
const PADDING_X: f32 = 10.0;
const FONT_SIZE: f32 = 13.0;
const DETAIL_FONT_SIZE: f32 = 12.0;
const DETAIL_PADDING: f32 = 8.0;
const POPUP_GAP: f32 = 4.0;
/// Popups have no OS shadow (see `platform::remove_window_shadows`), so a faint edge separates them.
const POPUP_BORDER: Stroke = Stroke { width: 1.0, color: Color32::from_gray(70) };
const GRAPH_HEIGHT: f32 = 36.0;
const GRAPH_MIN_WIDTH: f32 = 180.0;
/// Teal, matching the app icon.
const GRAPH_COLOR: Color32 = Color32::from_rgb(90, 200, 160);
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
/// How often the strip re-asserts its place at the top of the always-on-top band.
const TOPMOST_INTERVAL: Duration = Duration::from_secs(5);
/// Characters of the busiest app's name shown on the strip.
const TOP_NAME_WIDTH: usize = 10;

pub struct App {
    config: Config,
    /// Last state written to disk; differs from `config` while a save is pending.
    saved: Config,
    dirty_since: Option<Instant>,
    shared: Arc<Mutex<Shared>>,
    menu: Option<MenuState>,
    window: Option<Window>,
    controls: Option<Controls>,
    /// Mirrors the registry; the source of truth is the Run key, which the tray may also change.
    autostart: bool,
    /// Click-through state last sent to the window and the tray, to apply changes once.
    applied: Option<(bool, bool)>,
    raised_at: Instant,
}

struct MenuState {
    anchor: Anchor,
    x: f32,
    /// Starts at `MENU_SIZE` and becomes the measured size, which also re-places the menu so it
    /// never hangs over the strip or off the screen.
    size: Vec2,
    /// The menu closes on focus loss, but only after it has had focus once.
    was_focused: bool,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, config: Config) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        let shared = sampler::spawn(cc.egui_ctx.clone(), Settings::from_config(&config));

        let window = Window::from_handle(cc);
        let autostart = platform::autostart_enabled();
        if autostart {
            // Re-register so the entry follows the exe if it was moved.
            if let Err(err) = platform::set_autostart(true) {
                eprintln!("compteur: failed to refresh autostart: {err}");
            }
        }
        let controls =
            window.and_then(|window| Controls::new(cc.egui_ctx.clone(), window, config.click_through, autostart));

        Self {
            saved: config.clone(),
            config,
            dirty_since: None,
            shared,
            menu: None,
            window,
            controls,
            autostart,
            applied: None,
            raised_at: Instant::now(),
        }
    }

    /// Opens the settings menu `x` points from the strip's left edge.
    fn open_menu(&mut self, anchor: Anchor, x: f32) {
        self.menu = Some(MenuState { anchor, x, size: MENU_SIZE, was_focused: false });
    }

    /// Applies tray-menu and hotkey actions. Showing/hiding and autostart were already done by the
    /// event handler itself, since a hidden window gets no frames.
    fn handle_actions(&mut self, ctx: &egui::Context) {
        let actions = self.controls.as_ref().map(Controls::take_actions).unwrap_or_default();
        for action in actions {
            match action {
                Action::ToggleVisible => {}
                Action::ToggleClickThrough => self.config.click_through = !self.config.click_through,
                Action::OpenSettings => self.open_menu(Anchor::of(ctx, self.window), 0.0),
                Action::ToggleAutostart => self.autostart = platform::autostart_enabled(),
                Action::Exit => {
                    self.save_now();
                    ctx.send_viewport_cmd(ViewportCommand::Close);
                }
            }
        }

        let state = (self.config.click_through, self.autostart);
        if self.applied != Some(state) {
            ctx.send_viewport_cmd(ViewportCommand::MousePassthrough(self.config.click_through));
            if let Some(controls) = &self.controls {
                controls.sync(self.config.click_through, self.autostart);
            }
            self.applied = Some(state);
        }

        if let Some(window) = self.window {
            window.keep_out_of_alt_tab();
            if self.raised_at.elapsed() >= TOPMOST_INTERVAL {
                window.raise_topmost();
                self.raised_at = Instant::now();
            }
        }
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
        let mut autostart = self.autostart;
        let position = menu.anchor.popup_position(menu.x, menu.size);
        let builder = popup_builder("Compteur menu", position, menu.size);
        ctx.show_viewport_immediate(ViewportId::from_hash_of("menu"), builder, |ui, _class| {
            let focused = ui.ctx().input(|i| i.viewport().focused);
            menu.was_focused |= focused == Some(true);
            if (menu.was_focused && focused == Some(false))
                || ui.input(|i| i.key_pressed(egui::Key::Escape) || i.viewport().close_requested())
            {
                close = true;
            }
            let panel = egui::CentralPanel::default().show(ui, |ui| {
                exit = menu_contents(ui, config, &mut autostart);
                ui.min_rect().size()
            });
            ui.painter().rect_stroke(ui.max_rect(), CornerRadius::ZERO, POPUP_BORDER, StrokeKind::Inside);
            // Fit the window to its contents plus the panel's 8 pt margin on each side.
            let wanted = (panel.inner + Vec2::splat(16.0)).ceil();
            if (menu.size - wanted).length() > 1.0 {
                menu.size = wanted;
                let position = menu.anchor.popup_position(menu.x, wanted);
                ui.ctx().send_viewport_cmd(ViewportCommand::InnerSize(wanted));
                ui.ctx().send_viewport_cmd(ViewportCommand::OuterPosition(position));
            }
        });

        if autostart != self.autostart {
            if let Err(err) = platform::set_autostart(autostart) {
                eprintln!("compteur: failed to change autostart: {err}");
            }
            self.autostart = platform::autostart_enabled();
        }
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
        self.handle_actions(&ctx);
        self.persist(&ctx);

        let mut shared = sampler::lock(&self.shared);
        shared.settings = Settings::from_config(&self.config);
        let segments: Vec<(Metric, Arc<Galley>)> = segments(&shared, self.config.shown)
            .into_iter()
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
        let menu_click = response.secondary_clicked().then(|| response.interact_pointer_pos().map_or(0.0, |p| p.x));

        // Details for the hovered segment, unless the menu or a drag is in progress.
        let hovered = response
            .hover_pos()
            .filter(|_| self.menu.is_none() && menu_click.is_none() && !response.dragged())
            .and_then(|pos| spans.iter().find(|(_, span)| span.contains(&pos.x)))
            .map(|(metric, span)| (*metric, span.start));
        let details = hovered.map(|(metric, x)| {
            (x, detail_lines(metric, &shared, self.config.refresh_ms), detail_history(metric, &shared))
        });
        drop(shared);

        if let Some(x) = menu_click {
            self.open_menu(Anchor::of(&ctx, self.window), x);
        }

        if let Some((x, lines, history)) = details {
            let galleys: Vec<_> = lines
                .into_iter()
                .map(|line| ui.painter().layout_no_wrap(line, FontId::monospace(DETAIL_FONT_SIZE), TEXT_COLOR))
                .collect();
            show_details(&ctx, Anchor::of(&ctx, self.window), x - PADDING_X, galleys, history);
        }
        self.show_menu(&ctx);
        // After the popups above, so a window opened this frame never shows the shadow.
        platform::remove_window_shadows();
    }
}

/// Settings menu. Changes apply immediately; returns true when Exit was clicked.
fn menu_contents(ui: &mut egui::Ui, config: &mut Config, autostart: &mut bool) -> bool {
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
    ui.checkbox(&mut config.click_through, "Click-through (Ctrl+Alt+M)")
        .on_hover_text("Clicks go to the window below. Use the tray icon or the hotkey to turn it off.");
    ui.checkbox(autostart, "Start with Windows");

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

fn percent_level(percent: Option<f32>) -> Level {
    percent.map_or(Level::Normal, Level::for_percent)
}

fn percent_color(percent: Option<f32>) -> Color32 {
    level_color(percent_level(percent))
}

/// A segment's label takes its value's warning color too, so a busy segment stands out at a glance.
fn label_color(level: Level) -> Color32 {
    if level == Level::Normal { LABEL_COLOR } else { level_color(level) }
}

fn percent_segment(label: &str, percent: Option<f32>) -> LayoutJob {
    let level = percent_level(percent);
    segment_job(&[(label, label_color(level)), (&format::percent(percent), level_color(level))])
}

/// A load and a temperature sharing one segment, e.g. `CPU 23% 45°`.
/// The outer `Option` says whether that part is shown; the inner one holds its value.
fn load_and_temp_segment(label: &str, load: Option<Option<f32>>, temp: Option<Option<f32>>) -> LayoutJob {
    let load_text = load.map(format::percent);
    // Next to a load, an unavailable temperature is left out; on its own it shows as `--°`.
    let temp_text = temp
        .filter(|t| t.is_some() || load.is_none())
        .map(|t| if load.is_some() { format!(" {}", format::celsius(t)) } else { format::celsius(t) });
    let load_level = percent_level(load.flatten());
    let temp_level = temp.flatten().map_or(Level::Normal, Level::for_celsius);

    let mut parts = vec![(label, label_color(load_level.max(temp_level)))];
    if let Some(text) = &load_text {
        parts.push((text, level_color(load_level)));
    }
    if let Some(text) = &temp_text {
        parts.push((text, level_color(temp_level)));
    }
    segment_job(&parts)
}

/// Pads or cuts `text` to exactly `width` characters, so the strip doesn't change width.
fn fixed_width(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        format!("{text:<width$}")
    } else {
        let cut: String = text.chars().take(width - 1).collect();
        format!("{cut}…")
    }
}

/// The visible strip segments, in strip order, each tagged with the metric its hover details describe.
fn segments(shared: &Shared, shown: Shown) -> Vec<(Metric, LayoutJob)> {
    let snapshot = &shared.snapshot;
    let gpu = snapshot.gpu.as_ref();
    let mut segments = Vec::new();

    if shown.cpu || shown.cpu_temp {
        let load = shown.cpu.then(|| snapshot.cpu.as_ref().map(|cpu| cpu.usage));
        let temp = shown.cpu_temp.then_some(snapshot.cpu_temp);
        let metric = if shown.cpu { Metric::Cpu } else { Metric::CpuTemp };
        segments.push((metric, load_and_temp_segment("CPU ", load, temp)));
    }
    if shown.memory {
        segments.push((Metric::Memory, percent_segment("RAM ", snapshot.memory.as_ref().map(|m| m.percent()))));
    }
    if shown.gpu || shown.gpu_temp {
        let load = shown.gpu.then(|| gpu.and_then(|g| g.usage()));
        let temp = shown.gpu_temp.then(|| gpu.and_then(|g| g.temperature()));
        let metric = if shown.gpu { Metric::Gpu } else { Metric::GpuTemp };
        segments.push((metric, load_and_temp_segment("GPU ", load, temp)));
    }
    if shown.vram {
        segments.push((Metric::Vram, percent_segment("VRAM ", gpu.and_then(|g| g.main_adapter()?.vram_percent()))));
    }
    if shown.network {
        let network = snapshot.network.as_ref();
        segments.push((
            Metric::Network,
            segment_job(&[
                ("NET ↓", LABEL_COLOR),
                (&format::rate(network.map(|n| n.down_per_sec)), TEXT_COLOR),
                (" ↑", LABEL_COLOR),
                (&format::rate(network.map(|n| n.up_per_sec)), TEXT_COLOR),
            ]),
        ));
    }
    if shown.disk {
        let disk = snapshot.disk.as_ref();
        // Disk rates are colored by how busy the disks are, not by throughput.
        let disk_level = percent_level(disk.and_then(|d| d.active_percent));
        let disk_color = level_color(disk_level);
        segments.push((
            Metric::Disk,
            segment_job(&[
                ("DSK R", label_color(disk_level)),
                (&format::rate(disk.and_then(|d| d.read_per_sec)), disk_color),
                (" W", label_color(disk_level)),
                (&format::rate(disk.and_then(|d| d.write_per_sec)), disk_color),
            ]),
        ));
    }
    if shown.top_process {
        let top = snapshot.processes.as_ref().and_then(|p| p.first());
        let name = fixed_width(top.map_or("--", |p| p.name.as_str()), TOP_NAME_WIDTH);
        let cpu = top.map(|p| p.cpu);
        segments.push((
            Metric::TopProcess,
            segment_job(&[
                ("TOP ", LABEL_COLOR),
                (&name, TEXT_COLOR),
                (" ", LABEL_COLOR),
                (&format::percent(cpu), percent_color(cpu)),
            ]),
        ));
    }
    segments
}

fn peak(history: &History) -> f32 {
    history.iter().fold(0.0, f32::max)
}

fn detail_lines(metric: Metric, shared: &Shared, refresh_ms: u64) -> Vec<String> {
    let window_secs = HISTORY_LEN as u64 * refresh_ms / 1000;
    let snapshot = &shared.snapshot;
    match metric {
        Metric::Cpu | Metric::CpuTemp => {
            let mut lines = Vec::new();
            if let Some(cpu) = &snapshot.cpu {
                lines.push(format!("CPU {:.0}% at {:.2} GHz", cpu.usage, cpu.frequency_mhz as f32 / 1000.0));
                lines.push(format!("Peak, last {window_secs}s: {:.0}%", peak(&shared.cpu_history)));
            }
            lines.push(match snapshot.cpu_temp {
                Some(celsius) => format!("Temperature: {celsius:.0}°C (ACPI thermal zone)"),
                None => "Temperature: not exposed by this PC without a driver".to_owned(),
            });
            if let Some(cpu) = &snapshot.cpu {
                lines.push(format!("{} logical cores (%):", cpu.per_core.len()));
                lines.extend(
                    cpu.per_core
                        .chunks(8)
                        .map(|row| row.iter().map(|u| format!("{u:>3.0}")).collect::<Vec<_>>().join(" ")),
                );
            }
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
        Metric::Gpu | Metric::GpuTemp | Metric::Vram => {
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
                if let Some(celsius) = adapter.temperature {
                    lines.push(format!("  Temperature {celsius:.0}°C"));
                }
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
        Metric::TopProcess => {
            let Some(top) = &snapshot.processes else { return vec!["Top apps: measuring…".to_owned()] };
            let mut lines = vec!["Top apps by CPU     CPU  Memory".to_owned()];
            lines.extend(top.iter().map(|p| {
                let name = if p.count > 1 { format!("{} ({})", p.name, p.count) } else { p.name.clone() };
                format!("  {} {:>4.0}%  {:>6}", fixed_width(&name, 16), p.cpu, format::bytes(p.memory))
            }));
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

/// What popups are placed against: the strip on screen and the usable area of its monitor
/// (taskbar excluded), both in points.
#[derive(Debug, Clone, Copy)]
struct Anchor {
    strip: Rect,
    screen: Option<Rect>,
}

impl Anchor {
    fn of(ctx: &egui::Context, window: Option<Window>) -> Self {
        let (outer, scale) = ctx.input(|i| (i.viewport().outer_rect, i.viewport().native_pixels_per_point));
        let strip = outer.unwrap_or(Rect::from_min_size(Pos2::ZERO, Vec2::new(0.0, STRIP_HEIGHT)));
        let scale = scale.unwrap_or(1.0);
        let screen = window.and_then(Window::work_area).map(|[left, top, right, bottom]| {
            Rect::from_min_max(Pos2::new(left as f32, top as f32), Pos2::new(right as f32, bottom as f32)) / scale
        });
        Self { strip, screen }
    }

    /// Top-left of a popup of `size` aligned to `x` (relative to the strip's left edge): below the
    /// strip, or above it when it doesn't fit below, then slid back onto the screen if it would
    /// still stick out at any edge.
    fn popup_position(self, x: f32, size: Vec2) -> Pos2 {
        let below = self.strip.bottom() + POPUP_GAP;
        let above = self.strip.top() - POPUP_GAP - size.y;
        let mut position = Pos2::new(self.strip.left() + x, below);
        if let Some(screen) = self.screen {
            if below + size.y > screen.bottom() && above >= screen.top() {
                position.y = above;
            }
            // `min` before `max`, so a popup larger than the screen keeps its top-left corner visible.
            position.x = position.x.min(screen.right() - size.x).max(screen.left());
            position.y = position.y.min(screen.bottom() - size.y).max(screen.top());
        }
        position
    }
}

/// Shows `lines` next to the strip, aligned to `x` and kept on screen.
/// `history` (0–100 values, oldest first) is drawn as a graph under the text.
fn show_details(ctx: &egui::Context, anchor: Anchor, x: f32, lines: Vec<Arc<Galley>>, history: Option<Vec<f32>>) {
    let text_width = lines.iter().map(|g| g.size().x).fold(0.0, f32::max);
    let text_height: f32 = lines.iter().map(|g| g.size().y).sum();
    let width = if history.is_some() { text_width.max(GRAPH_MIN_WIDTH) } else { text_width };
    let height = text_height + if history.is_some() { DETAIL_PADDING + GRAPH_HEIGHT } else { 0.0 };
    let size = Vec2::new(width, height) + Vec2::splat(2.0 * DETAIL_PADDING);

    let size = size.ceil();
    let builder = popup_builder("Compteur details", anchor.popup_position(x, size), size)
        .with_active(false)
        .with_mouse_passthrough(true);

    ctx.show_viewport_immediate(ViewportId::from_hash_of("details"), builder, |ui, _class| {
        let rect = ui.max_rect();
        ui.painter().rect_filled(rect, CornerRadius::ZERO, Color32::from_gray(24));
        ui.painter().rect_stroke(rect, CornerRadius::ZERO, POPUP_BORDER, StrokeKind::Inside);
        let mut pos = rect.min + Vec2::splat(DETAIL_PADDING);
        for galley in lines.clone() {
            let height = galley.size().y;
            ui.painter().galley(pos, galley, TEXT_COLOR);
            pos.y += height;
        }
        if let Some(history) = &history {
            let graph = Rect::from_min_size(pos + Vec2::new(0.0, DETAIL_PADDING), Vec2::new(width, GRAPH_HEIGHT));
            draw_graph(ui.painter(), graph, history);
        }
    });
}

/// The 0–100% history graphed under a metric's details, if it has one worth drawing.
fn detail_history(metric: Metric, shared: &Shared) -> Option<Vec<f32>> {
    let history = match metric {
        Metric::Cpu | Metric::CpuTemp => &shared.cpu_history,
        Metric::Memory => &shared.memory_history,
        Metric::Gpu | Metric::GpuTemp => &shared.gpu_history,
        Metric::Vram | Metric::Network | Metric::Disk | Metric::TopProcess => return None,
    };
    (history.iter().len() >= 2).then(|| history.iter().collect())
}

/// A 0–100% line graph with the newest value at the right edge, so a fresh history fills in from the right.
fn draw_graph(painter: &egui::Painter, rect: Rect, values: &[f32]) {
    let grid = Stroke::new(1.0, Color32::from_gray(60));
    painter.rect_stroke(rect, CornerRadius::ZERO, grid, StrokeKind::Inside);
    painter.hline(rect.x_range(), rect.center().y, Stroke::new(1.0, Color32::from_gray(40))); // 50%

    let step = rect.width() / (HISTORY_LEN - 1) as f32;
    let points: Vec<Pos2> = values
        .iter()
        .rev()
        .enumerate()
        .map(|(age, value)| {
            let y = rect.bottom() - value.clamp(0.0, 100.0) / 100.0 * rect.height();
            Pos2::new(rect.right() - age as f32 * step, y)
        })
        .collect();
    painter.add(egui::Shape::line(points, Stroke::new(1.5, GRAPH_COLOR)));
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: Rect = Rect { min: Pos2::new(0.0, 0.0), max: Pos2::new(1000.0, 800.0) };
    const MENU: Vec2 = Vec2::new(200.0, 300.0);

    fn anchor_at(left: f32, top: f32) -> Anchor {
        Anchor {
            strip: Rect::from_min_size(Pos2::new(left, top), Vec2::new(400.0, STRIP_HEIGHT)),
            screen: Some(SCREEN),
        }
    }

    #[test]
    fn popup_goes_below_the_strip_when_it_fits() {
        assert_eq!(
            anchor_at(100.0, 50.0).popup_position(30.0, MENU),
            Pos2::new(130.0, 50.0 + STRIP_HEIGHT + POPUP_GAP)
        );
    }

    #[test]
    fn popup_goes_above_a_strip_near_the_bottom() {
        let top = 760.0;
        assert_eq!(anchor_at(100.0, top).popup_position(0.0, MENU).y, top - POPUP_GAP - MENU.y);
    }

    #[test]
    fn popup_slides_left_at_the_right_edge() {
        assert_eq!(anchor_at(700.0, 50.0).popup_position(250.0, MENU).x, SCREEN.right() - MENU.x);
    }

    #[test]
    fn popup_taller_than_both_sides_stays_on_screen() {
        let position = anchor_at(0.0, 300.0).popup_position(0.0, Vec2::new(200.0, 600.0));
        assert!(position.y >= 0.0 && position.y + 600.0 <= SCREEN.bottom());
    }

    #[test]
    fn popup_without_screen_info_goes_below() {
        let anchor = Anchor { screen: None, ..anchor_at(10.0, 790.0) };
        assert_eq!(anchor.popup_position(0.0, MENU), Pos2::new(10.0, 790.0 + STRIP_HEIGHT + POPUP_GAP));
    }
}
