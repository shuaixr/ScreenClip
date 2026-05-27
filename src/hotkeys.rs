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

    #[cfg(target_os = "linux")]
    warn!(
        "global PrintScreen registration can be blocked by Wayland or the desktop environment; configure a compositor shortcut if needed"
    );

    #[cfg(target_os = "macos")]
    warn!(
        "macOS may require Accessibility/Input Monitoring permissions for global hotkeys and Screen Recording permission for capture"
    );
}
