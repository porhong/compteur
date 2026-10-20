use std::time::{Duration, Instant};

use eframe::egui::{
    self, Color32, CornerRadius, FontId, Id, PointerButton, Pos2, Sense, Vec2, ViewportBuilder, ViewportCommand,
    ViewportId,
};

use crate::config::Config;

pub const STRIP_HEIGHT: f32 = 24.0;
const PADDING_X: f32 = 10.0;
const FONT_SIZE: f32 = 13.0;
const TEXT_COLOR: Color32 = Color32::from_gray(230);
/// Wait for the window to stop moving before writing the config to disk.
const SAVE_DELAY: Duration = Duration::from_millis(800);

pub struct App {
    config: Config,
    /// Last state written to disk; differs from `config` while a save is pending.
    saved: Config,
    dirty_since: Option<Instant>,
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
        Self { saved: config.clone(), config, dirty_since: None, menu: None }
    }

    fn save_now(&mut self) {
        if let Err(err) = self.config.save() {
            eprintln!("compteur: failed to save config: {err}");
        }
        self.saved = self.config.clone();
        self.dirty_since = None;
    }

    /// Records the window position and saves once it has been stable for `SAVE_DELAY`.
    fn persist_position(&mut self, ctx: &egui::Context) {
        let position = ctx.input(|i| i.viewport().outer_rect).map(|r| [r.min.x.round(), r.min.y.round()]);
        if position.is_some() && position != self.config.position {
            self.config.position = position;
            self.dirty_since = Some(Instant::now());
        }
        if let Some(since) = self.dirty_since {
            if since.elapsed() >= SAVE_DELAY {
                self.save_now();
            } else {
                ctx.request_repaint_after(SAVE_DELAY - since.elapsed());
            }
        }
    }

    /// Shrinks or grows the window to fit the strip's content.
    fn fit_window(ctx: &egui::Context, size: Vec2) {
        let current = ctx.input(|i| i.viewport().inner_rect).map(|r| r.size());
        if current.is_none_or(|current| (current - size).length() > 0.5) {
            ctx.send_viewport_cmd(ViewportCommand::InnerSize(size));
        }
    }

    /// The strip is too small to host a popup, so the menu is its own borderless window.
    fn show_menu(&mut self, ctx: &egui::Context) {
        let Some(menu) = &mut self.menu else { return };
        let mut close = false;
        let mut exit = false;

        let builder = ViewportBuilder::default()
            .with_title("Compteur menu")
            .with_position(menu.position)
            .with_inner_size([160.0, 40.0])
            .with_decorations(false)
            .with_resizable(false)
            .with_window_level(egui::WindowLevel::AlwaysOnTop)
            .with_taskbar(false);
        ctx.show_viewport_immediate(ViewportId::from_hash_of("menu"), builder, |ui, _class| {
            let focused = ui.ctx().input(|i| i.viewport().focused);
            menu.was_focused |= focused == Some(true);
            if (menu.was_focused && focused == Some(false))
                || ui.input(|i| i.key_pressed(egui::Key::Escape) || i.viewport().close_requested())
            {
                close = true;
            }
            egui::CentralPanel::default().show(ui, |ui| {
                if ui.button("Exit Compteur").clicked() {
                    exit = true;
                }
            });
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
        self.persist_position(&ctx);

        let text = "CPU --% │ RAM --% │ GPU --% │ ↓-- ↑-- │ D --/--";
        let galley = ui.painter().layout_no_wrap(text.to_owned(), FontId::monospace(FONT_SIZE), TEXT_COLOR);
        let size = Vec2::new((galley.size().x + 2.0 * PADDING_X).ceil(), STRIP_HEIGHT);
        Self::fit_window(&ctx, size);

        let rect = egui::Rect::from_min_size(ui.max_rect().min, size);
        let background = Color32::from_black_alpha((self.config.opacity * 255.0) as u8);
        ui.painter().rect_filled(rect, CornerRadius::same(6), background);
        let text_pos = rect.left_center() + Vec2::new(PADDING_X, -galley.size().y / 2.0);
        ui.painter().galley(text_pos, galley, TEXT_COLOR);

        let response = ui.interact(rect, Id::unique("strip"), Sense::click_and_drag());
        if response.drag_started_by(PointerButton::Primary) {
            ctx.send_viewport_cmd(ViewportCommand::StartDrag);
        }
        if response.secondary_clicked() {
            let origin = ctx.input(|i| i.viewport().outer_rect).map_or(Pos2::ZERO, |r| r.min);
            let click = response.interact_pointer_pos().unwrap_or(rect.left_bottom());
            self.menu = Some(MenuState { position: origin + click.to_vec2(), was_focused: false });
        }
        self.show_menu(&ctx);
    }
}
