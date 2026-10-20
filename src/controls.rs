//! Tray icon and global hotkeys: the ways back to the strip when it is click-through or hidden.

use std::sync::mpsc::{self, Receiver};

use eframe::egui;
use global_hotkey::{
    GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState,
    hotkey::{Code, HotKey, Modifiers},
};
use tray_icon::{
    Icon, TrayIcon, TrayIconBuilder,
    menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem},
};

use crate::{icon::icon_rgba, platform, platform::Window};

const TRAY_ICON_SIZE: u32 = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    ToggleVisible,
    ToggleClickThrough,
    OpenSettings,
    ToggleAutostart,
    Exit,
}

pub struct Controls {
    _tray: TrayIcon,
    /// Kept alive so the hotkeys stay registered.
    _hotkeys: Option<GlobalHotKeyManager>,
    click_through_item: CheckMenuItem,
    autostart_item: CheckMenuItem,
    actions: Receiver<Action>,
}

impl Controls {
    /// Must run on the UI thread, whose message loop drives the tray and hotkeys.
    pub fn new(ctx: egui::Context, window: Window, click_through: bool, autostart: bool) -> Option<Self> {
        // A plain item, not a check mark: while hidden the UI gets no frames to keep a check mark in sync.
        let visible_item = MenuItem::new("Show / hide strip\tCtrl+Alt+H", true, None);
        let click_through_item = CheckMenuItem::new("Click-through\tCtrl+Alt+M", true, click_through, None);
        let settings_item = MenuItem::new("Settings…", true, None);
        let autostart_item = CheckMenuItem::new("Start with Windows", true, autostart, None);
        let exit_item = MenuItem::new("Exit", true, None);
        let menu = Menu::new();
        menu.append_items(&[
            &visible_item,
            &click_through_item,
            &settings_item,
            &autostart_item,
            &PredefinedMenuItem::separator(),
            &exit_item,
        ])
        .ok()?;

        let icon = Icon::from_rgba(icon_rgba(TRAY_ICON_SIZE), TRAY_ICON_SIZE, TRAY_ICON_SIZE).ok()?;
        let tray = TrayIconBuilder::new()
            .with_icon(icon)
            .with_tooltip("Compteur")
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(true)
            .build()
            .ok()?;

        let (sender, actions) = mpsc::channel();
        let dispatch = move |action: Action| {
            // A hidden egui window gets no frames, so anything that can't wait for one happens right here.
            match action {
                Action::ToggleVisible => window.set_visible(!window.is_visible()),
                Action::OpenSettings | Action::Exit => window.set_visible(true),
                Action::ToggleAutostart => {
                    if let Err(err) = platform::set_autostart(!platform::autostart_enabled()) {
                        eprintln!("compteur: failed to change autostart: {err}");
                    }
                }
                Action::ToggleClickThrough => {}
            }
            let _ = sender.send(action); // the receiver only goes away at exit
            ctx.request_repaint();
        };

        let menu_ids = [
            (visible_item.id().clone(), Action::ToggleVisible),
            (click_through_item.id().clone(), Action::ToggleClickThrough),
            (settings_item.id().clone(), Action::OpenSettings),
            (autostart_item.id().clone(), Action::ToggleAutostart),
            (exit_item.id().clone(), Action::Exit),
        ];
        let menu_dispatch = dispatch.clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            if let Some((_, action)) = menu_ids.iter().find(|(id, _)| *id == event.id) {
                menu_dispatch(*action);
            }
        }));

        let hotkeys = register_hotkeys(dispatch);
        Some(Self {
            _tray: tray,
            _hotkeys: hotkeys,
            click_through_item,
            autostart_item,
            actions,
        })
    }

    /// Actions from the tray menu and hotkeys since the last call.
    pub fn take_actions(&self) -> Vec<Action> {
        self.actions.try_iter().collect()
    }

    /// Updates the check marks after changes made elsewhere (hotkeys, the settings menu).
    pub fn sync(&self, click_through: bool, autostart: bool) {
        self.click_through_item.set_checked(click_through);
        self.autostart_item.set_checked(autostart);
    }
}

/// `None` if the hotkeys couldn't be registered (e.g. another app already owns them); the tray still works.
fn register_hotkeys(dispatch: impl Fn(Action) + Send + Sync + 'static) -> Option<GlobalHotKeyManager> {
    let manager = GlobalHotKeyManager::new().ok()?;
    let modifiers = Some(Modifiers::CONTROL | Modifiers::ALT);
    let bindings = [
        (HotKey::new(modifiers, Code::KeyH), Action::ToggleVisible),
        (HotKey::new(modifiers, Code::KeyM), Action::ToggleClickThrough),
    ];
    for (hotkey, _) in &bindings {
        if let Err(err) = manager.register(*hotkey) {
            eprintln!("compteur: hotkey unavailable: {err}");
        }
    }
    let ids = bindings.map(|(hotkey, action)| (hotkey.id(), action));
    GlobalHotKeyEvent::set_event_handler(Some(move |event: GlobalHotKeyEvent| {
        if event.state == HotKeyState::Pressed
            && let Some((_, action)) = ids.iter().find(|(id, _)| *id == event.id)
        {
            dispatch(*action);
        }
    }));
    Some(manager)
}
