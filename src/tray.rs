use std::error::Error;

use log::{info, warn};
use screenshots::image::{self, ImageFormat};
use tray_icon::{
    menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem},
    Icon, TrayIcon, TrayIconBuilder,
};
use winit::event_loop::EventLoopProxy;

use crate::AppEvent;

pub(crate) struct TrayRuntime {
    _tray_icon: TrayIcon,
    _menu: Menu,
    _capture_item: MenuItem,
    _autostart_item: CheckMenuItem,
    _quit_item: MenuItem,
}

impl TrayRuntime {
    pub(crate) fn new(proxy: EventLoopProxy<AppEvent>) -> Result<Self, Box<dyn Error>> {
        let capture_item = MenuItem::new("Capture screenshot", true, None);
        let separator = PredefinedMenuItem::separator();
        let autostart_item =
            CheckMenuItem::new("Launch at startup", true, is_autostart_enabled(), None);
        let separator2 = PredefinedMenuItem::separator();
        let quit_item = MenuItem::new("Quit", true, None);
        let menu = Menu::with_items(&[
            &capture_item,
            &separator,
            &autostart_item,
            &separator2,
            &quit_item,
        ])?;

        let capture_id = capture_item.id().clone();
        let autostart_id = autostart_item.id().clone();
        let quit_id = quit_item.id().clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            if event.id == capture_id {
                let _ = proxy.send_event(AppEvent::CaptureRequested);
            } else if event.id == autostart_id {
                // The CheckMenuItem has already toggled its checked state; read it back
                // via is_autostart_enabled after we apply the change, but we derive the
                // desired state from the *current* registry state and invert it.
                let _ = proxy.send_event(AppEvent::ToggleAutostart);
            } else if event.id == quit_id {
                let _ = proxy.send_event(AppEvent::QuitRequested);
            }
        }));

        let tray_icon = TrayIconBuilder::new()
            .with_tooltip("Screenshot listener")
            .with_menu(Box::new(menu.clone()))
            .with_icon(app_icon()?)
            .build()?;

        info!("system tray icon created");
        Ok(Self {
            _tray_icon: tray_icon,
            _menu: menu,
            _capture_item: capture_item,
            _autostart_item: autostart_item,
            _quit_item: quit_item,
        })
    }
}

/// Returns true if the app is registered in the OS autostart mechanism.
pub(crate) fn is_autostart_enabled() -> bool {
    build_auto_launch()
        .map(|al| al.is_enabled().unwrap_or(false))
        .unwrap_or(false)
}

/// Enables or disables OS autostart for the app.
pub(crate) fn set_autostart(enabled: bool) {
    match build_auto_launch() {
        Ok(al) => {
            let result = if enabled { al.enable() } else { al.disable() };
            if let Err(e) = result {
                warn!(
                    "failed to {} autostart: {e}",
                    if enabled { "enable" } else { "disable" }
                );
            } else {
                info!("autostart {}", if enabled { "enabled" } else { "disabled" });
            }
        }
        Err(e) => warn!("failed to build AutoLaunch: {e}"),
    }
}

fn build_auto_launch() -> Result<auto_launch::AutoLaunch, auto_launch::Error> {
    let exe_path = std::env::current_exe()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    auto_launch::AutoLaunchBuilder::new()
        .set_app_name("ScreenClip")
        .set_app_path(&exe_path)
        .build()
}

pub(crate) fn log_install_error(err: &(dyn Error + 'static)) {
    warn!("failed to create system tray icon: {err}");

    #[cfg(target_os = "linux")]
    warn!(
        "Linux tray icons require GTK and an appindicator implementation such as libappindicator or libayatana-appindicator"
    );
}

fn app_icon() -> Result<Icon, Box<dyn Error>> {
    let icon = image::load_from_memory_with_format(
        include_bytes!("../assets/icons/icons.png"),
        ImageFormat::Png,
    )
    .map_err(|err| format!("failed to decode embedded icon PNG: {err}"))?
    .to_rgba8();
    let (width, height) = icon.dimensions();
    Icon::from_rgba(icon.into_raw(), width, height)
        .map_err(|err| format!("failed to build tray icon RGBA: {err}").into())
}
