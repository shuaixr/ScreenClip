use std::thread;

#[cfg(target_os = "macos")]
use global_hotkey::hotkey::Modifiers;
use global_hotkey::{
    hotkey::{Code, HotKey},
    GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState,
};
use log::{info, warn};
use winit::event_loop::EventLoopProxy;

use crate::AppEvent;

pub(crate) struct HotkeyRuntime {
    _manager: GlobalHotKeyManager,
    _screenshot_hotkey: HotKey,
    _listener: thread::JoinHandle<()>,
}

impl HotkeyRuntime {
    pub(crate) fn new(proxy: EventLoopProxy<AppEvent>) -> Result<Self, global_hotkey::Error> {
        let manager = GlobalHotKeyManager::new()?;
        let screenshot_hotkey = screenshot_hotkey();
        manager.register(screenshot_hotkey.clone())?;

        let hotkey_id = screenshot_hotkey.id();
        let listener = thread::Builder::new()
            .name("screenshot-hotkey-listener".to_string())
            .spawn(move || {
                let receiver = GlobalHotKeyEvent::receiver();
                while let Ok(event) = receiver.recv() {
                    if event.id == hotkey_id && event.state == HotKeyState::Pressed {
                        if proxy.send_event(AppEvent::CaptureRequested).is_err() {
                            break;
                        }
                    }
                }
            })
            .expect("failed to spawn hotkey listener");

        info!(
            "registered screenshot hotkey: {}",
            screenshot_hotkey_label()
        );
        Ok(Self {
            _manager: manager,
            _screenshot_hotkey: screenshot_hotkey,
            _listener: listener,
        })
    }
}

#[cfg(target_os = "macos")]
fn screenshot_hotkey() -> HotKey {
    HotKey::new(Some(Modifiers::SUPER | Modifiers::SHIFT), Code::KeyS)
}

#[cfg(not(target_os = "macos"))]
fn screenshot_hotkey() -> HotKey {
    HotKey::new(None, Code::PrintScreen)
}

#[cfg(target_os = "macos")]
fn screenshot_hotkey_label() -> &'static str {
    "Cmd+Shift+S"
}

#[cfg(not(target_os = "macos"))]
fn screenshot_hotkey_label() -> &'static str {
    "PrintScreen"
}

pub(crate) fn log_install_error(err: &global_hotkey::Error) {
    warn!("failed to register screenshot hotkey: {err}");

    log_registration_hints();
}

pub(crate) fn log_registration_succeeded() {
    // The crate's `register()` returning Ok is not a guarantee that the OS
    // will actually deliver the key event — on Wayland, registration is
    // silently rejected by the compositor for most keys. Surface a hint
    // when we detect a Wayland session even though registration nominally
    // succeeded, so the user is not left wondering why PrintScreen does
    // nothing.
    log_registration_hints();
}

fn log_registration_hints() {
    #[cfg(target_os = "linux")]
    {
        let on_wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
        let on_x11 = std::env::var_os("DISPLAY").is_some();
        if on_wayland {
            warn!(
                "running under Wayland; global hotkeys are typically blocked by the compositor. \
                 Bind a desktop shortcut to `screenclip --capture` (Settings → Keyboard → \
                 Custom Shortcuts) instead of relying on PrintScreen."
            );
        } else if !on_x11 {
            warn!(
                "no X11 display server detected; global hotkey registration is unlikely to work"
            );
        }
    }

    #[cfg(target_os = "macos")]
    warn!(
        "macOS may require Accessibility/Input Monitoring permissions for global hotkeys and Screen Recording permission for capture"
    );
}
