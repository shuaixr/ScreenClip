#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod annotations;
mod cpu_present;
mod cpu_renderer;
mod desktop_geometry;
mod hotkeys;
mod overlay_ui;
mod selection_geometry;
mod tray;

use std::{collections::HashMap, path::PathBuf, sync::Arc, time::Instant};

use ab_glyph::FontArc;
use annotations::rectangle::RectangleAnnotation;
use annotations::text::{
    TextAnnotation, TextAnnotationCursorHit, TextAnnotationDrag, TEXT_CARET_BLINK_INTERVAL,
    TEXT_LINE_HEIGHT_PIXELS,
};
use arboard::{Clipboard, ImageData};
use cpu_present::CpuPresenter;
use cpu_renderer::CpuRenderer;
use log::{debug, info, warn};
use overlay_ui::{OverlayAction, OverlayUi, ToolMode};
use screenshots::image::{imageops::FilterType, Rgba, RgbaImage};
use screenshots::Screen;
use selection_geometry::ResizeEdge;
use winit::{
    application::ApplicationHandler,
    dpi::{PhysicalPosition, PhysicalSize, Position, Size},
    event::{ElementState, MouseButton, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy},
    keyboard::{Key, NamedKey},
    monitor::MonitorHandle,
    window::{Cursor, CursorIcon, Window, WindowAttributes, WindowId, WindowLevel},
};

use std::borrow::Cow;

#[cfg(target_os = "windows")]
use windows_sys::Win32::{
    Foundation::{GetLastError, ERROR_ACCESS_DENIED, ERROR_INVALID_HANDLE},
    System::Console::{AttachConsole, ATTACH_PARENT_PROCESS},
    UI::WindowsAndMessaging::{
        SystemParametersInfoW, ANIMATIONINFO, SPI_GETANIMATION, SPI_SETANIMATION,
        SPIF_SENDCHANGE,
    },
};

#[cfg(target_os = "windows")]
use winit::platform::windows::WindowAttributesExtWindows;

struct WindowEgui {
    ctx: egui::Context,
    state: egui_winit::State,
    ui: OverlayUi,
}

struct WindowState {
    window: Arc<Window>,
    origin: PhysicalPosition<i32>,
    size: PhysicalSize<u32>,
    source_screenshot: RgbaImage,
    screenshot: RgbaImage,
    presenter: CpuPresenter,
    renderer: CpuRenderer,
    needs_show: bool,
    egui: WindowEgui,
}

struct CapturedMonitor {
    origin: PhysicalPosition<i32>,
    image: RgbaImage,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum AppEvent {
    CaptureRequested,
    QuitRequested,
    ToggleAutostart,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SelectionResizeDrag {
    edge: ResizeEdge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EscapeAction {
    ExitTextMode,
    ExitRectangleMode,
    CloseSession,
}

impl EscapeAction {
    fn for_overlay_state(has_active_session: bool, tool_mode: ToolMode) -> Option<Self> {
        if !has_active_session {
            return None;
        }

        match tool_mode {
            ToolMode::Text => Some(Self::ExitTextMode),
            ToolMode::Rectangle => Some(Self::ExitRectangleMode),
            ToolMode::Select => Some(Self::CloseSession),
        }
    }
}

struct App {
    initialized: bool,
    event_proxy: EventLoopProxy<AppEvent>,
    hotkeys: Option<hotkeys::HotkeyRuntime>,
    tray: Option<tray::TrayRuntime>,
    windows: HashMap<WindowId, WindowState>,
    last_cursor_global: Option<(i32, i32)>,
    drag_start: Option<(i32, i32)>,
    drag_current: Option<(i32, i32)>,
    selected_rect: Option<(i32, i32, i32, i32)>,
    selection_resize: Option<SelectionResizeDrag>,
    captures: Vec<CapturedMonitor>,
    tool_mode: ToolMode,
    text_annotations: Vec<TextAnnotation>,
    rectangle_annotations: Vec<RectangleAnnotation>,
    next_text_annotation_id: u64,
    pending_text_focus_id: Option<u64>,
    text_font_bytes: Vec<(String, Vec<u8>)>,
    text_fonts: Vec<FontArc>,
    needs_redraw: bool,
    next_caret_redraw: Option<Instant>,
    focused_text_annotation_id: Option<u64>,
    text_annotation_drag: Option<TextAnnotationDrag>,
    /// True if `--capture` was passed on the command line: the next `resumed()`
    /// invocation will synthesize a `CaptureRequested` event and then clear
    /// this flag, so a single CLI invocation always triggers exactly one
    /// capture session regardless of how many times the OS resumes the app.
    capture_on_start: bool,
    /// The previous `SPI_GETANIMATION` value captured right before we disabled
    /// window animations for a capture session, so we can restore it afterwards.
    /// `None` means no restore is pending.
    #[cfg(target_os = "windows")]
    saved_animation_info: Option<ANIMATIONINFO>,
}

impl App {
    fn new(event_proxy: EventLoopProxy<AppEvent>, capture_on_start: bool) -> Self {
        Self {
            initialized: false,
            event_proxy,
            hotkeys: None,
            tray: None,
            windows: HashMap::new(),
            last_cursor_global: None,
            drag_start: None,
            drag_current: None,
            selected_rect: None,
            selection_resize: None,
            captures: Vec::new(),
            tool_mode: ToolMode::default(),
            text_annotations: Vec::new(),
            rectangle_annotations: Vec::new(),
            next_text_annotation_id: 0,
            pending_text_focus_id: None,
            text_font_bytes: Vec::new(),
            text_fonts: Vec::new(),
            needs_redraw: false,
            next_caret_redraw: None,
            focused_text_annotation_id: None,
            text_annotation_drag: None,
            capture_on_start,
            #[cfg(target_os = "windows")]
            saved_animation_info: None,
        }
    }

    fn begin_capture_session(&mut self, event_loop: &ActiveEventLoop) {
        if !self.windows.is_empty() {
            info!("screenshot hotkey ignored because a capture session is already active");
            return;
        }

        self.reset_capture_state();

        // Suppress the Windows "Show Animation" that plays when a window becomes
        // visible — otherwise our overlay scales/fades in instead of appearing
        // instantly, which looks like a flash on every launch.
        #[cfg(target_os = "windows")]
        {
            self.saved_animation_info = read_window_animation_setting();
            set_window_animation_enabled(false);
        }

        let monitors: Vec<MonitorHandle> = event_loop.available_monitors().collect();
        let screens = Screen::all().unwrap_or_default();
        info!("detected {} monitors", monitors.len());

        for (idx, monitor) in monitors.into_iter().enumerate() {
            let size = monitor.size();
            let pos = monitor.position();
            let monitor_for_window = monitor.clone();

            // Cover the monitor with a borderless, always-on-top window instead of
            // `Fullscreen::Borderless` — winit's fullscreen path runs
            // `force_window_active()` (SendInput of an Alt keypress) which makes
            // Windows render a system label with the window title at the top of
            // the focused monitor for one frame. See:
            //   - winit #4116 (borderless flicker on creation)
            //   - winit #3576 (DWMWA_CLOAK, the proper upstream fix)
            //   - winit platform_impl/windows/window.rs `force_window_active`
            let _ = monitor_for_window;
            let mut attrs = WindowAttributes::default()
                .with_title(format!("overlay-{}", idx))
                .with_decorations(false)
                .with_resizable(false)
                .with_window_level(WindowLevel::AlwaysOnTop)
                .with_position(Position::Physical(PhysicalPosition::new(pos.x, pos.y)))
                .with_inner_size(Size::Physical(PhysicalSize::new(size.width, size.height)))
                .with_visible(false);

            #[cfg(target_os = "windows")]
            {
                attrs = attrs.with_skip_taskbar(true);
            }

            let window = Arc::new(
                event_loop
                    .create_window(attrs)
                    .expect("create window failed"),
            );
            window.set_cursor(Cursor::Icon(CursorIcon::Crosshair));

            let image = capture_or_fallback(&screens, pos.x, pos.y, size.width, size.height);
            self.captures.push(CapturedMonitor {
                origin: pos,
                image: image.clone(),
            });

            let egui_ctx = self.create_egui_context();
            let egui_state = egui_winit::State::new(
                egui_ctx.clone(),
                egui::ViewportId::ROOT,
                &*window,
                Some(window.scale_factor() as f32),
                None,
                None,
            );

            let presenter = CpuPresenter::new(window.clone(), size.width, size.height)
                .expect("failed to init CPU presenter");

            let id = window.id();
            self.windows.insert(
                id,
                WindowState {
                    window: window.clone(),
                    origin: pos,
                    size: PhysicalSize::new(size.width.max(1), size.height.max(1)),
                    source_screenshot: image.clone(),
                    screenshot: image,
                    presenter,
                    renderer: CpuRenderer::new(size.width, size.height),
                    needs_show: true,
                    egui: WindowEgui {
                        ctx: egui_ctx,
                        state: egui_state,
                        ui: OverlayUi::new(),
                    },
                },
            );

            let current_monitor_name = window
                .current_monitor()
                .and_then(|m| m.name())
                .unwrap_or_else(|| "unknown".to_string());
            info!(
                "window {} created target=({}, {}) {}x{} actual_monitor={}",
                idx, pos.x, pos.y, size.width, size.height, current_monitor_name
            );
        }

        self.needs_redraw = true;
        event_loop.set_control_flow(ControlFlow::Wait);
    }

    fn end_capture_session(&mut self) {
        for state in self.windows.values() {
            state.window.set_visible(false);
        }
        self.reset_capture_state();

        // Restore the user's animation setting that we stashed at the start of
        // the capture session.
        #[cfg(target_os = "windows")]
        {
            if let Some(info) = self.saved_animation_info.take() {
                let restore = ANIMATIONINFO {
                    cbSize: info.cbSize,
                    iMinAnimate: info.iMinAnimate,
                };
                unsafe {
                    let ok = SystemParametersInfoW(
                        SPI_SETANIMATION,
                        restore.cbSize,
                        &restore as *const _ as *mut _,
                        SPIF_SENDCHANGE,
                    );
                    if ok == 0 {
                        warn!(
                            "failed to restore SPI_SETANIMATION (error {})",
                            GetLastError()
                        );
                    }
                }
            }
        }
    }

    fn reset_capture_state(&mut self) {
        self.windows.clear();
        self.captures.clear();
        self.last_cursor_global = None;
        self.drag_start = None;
        self.drag_current = None;
        self.selected_rect = None;
        self.selection_resize = None;
        self.tool_mode = ToolMode::Select;
        self.text_annotations.clear();
        self.rectangle_annotations.clear();
        self.next_text_annotation_id = 0;
        self.pending_text_focus_id = None;
        self.needs_redraw = false;
        self.next_caret_redraw = None;
        self.focused_text_annotation_id = None;
        self.text_annotation_drag = None;
    }

    fn escape_action(&self) -> Option<EscapeAction> {
        EscapeAction::for_overlay_state(!self.windows.is_empty(), self.tool_mode)
    }

    fn exit_text_mode(&mut self) {
        self.tool_mode = ToolMode::Select;
        self.pending_text_focus_id = None;
        self.focused_text_annotation_id = None;
        self.text_annotation_drag = None;
        self.next_caret_redraw = None;
        self.needs_redraw = true;
    }

    fn exit_rectangle_mode(&mut self) {
        self.tool_mode = ToolMode::Select;
        self.needs_redraw = true;
    }

    fn handle_escape(&mut self) -> bool {
        match self.escape_action() {
            Some(EscapeAction::ExitTextMode) => {
                self.exit_text_mode();
                true
            }
            Some(EscapeAction::ExitRectangleMode) => {
                self.exit_rectangle_mode();
                true
            }
            Some(EscapeAction::CloseSession) => {
                self.end_capture_session();
                true
            }
            None => false,
        }
    }

    fn create_egui_context(&self) -> egui::Context {
        let egui_ctx = egui::Context::default();
        let mut fonts = egui::FontDefinitions::default();
        for (font_index, (font_name, bytes)) in self.text_font_bytes.iter().enumerate() {
            let key = format!("overlay_text_font_{font_index}");
            fonts.font_data.insert(
                key.clone(),
                egui::FontData::from_owned(bytes.clone()).into(),
            );
            if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
                family.insert(0, key.clone());
            }
            if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Monospace) {
                family.insert(0, key);
            }
            info!("registered text font {}", font_name);
        }
        egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
        egui_ctx.set_fonts(fonts);
        egui_ctx.set_visuals(egui::Visuals::dark());
        egui_ctx
    }
}

fn init_console_ctrl_c_shutdown(event_proxy: EventLoopProxy<AppEvent>) {
    #[cfg(target_os = "windows")]
    if !attach_parent_console_if_available() {
        debug!("startup: no parent console detected, terminal Ctrl+C is inactive");
        return;
    }

    match ctrlc::set_handler(move || {
        let _ = event_proxy.send_event(AppEvent::QuitRequested);
    }) {
        // Only relevant when the app is launched from a terminal — the handler
        // forwards the terminal's SIGINT to a clean shutdown. Demoted to debug
        // because the message is easily mistaken for the app hijacking Ctrl+C
        // system-wide, which it does not.
        Ok(()) => debug!("startup: terminal SIGINT handler registered"),
        Err(err) => warn!("startup: failed to register terminal SIGINT handler: {err}"),
    }
}

#[cfg(target_os = "windows")]
fn attach_parent_console_if_available() -> bool {
    unsafe {
        if AttachConsole(ATTACH_PARENT_PROCESS) != 0 {
            info!("startup: attached to parent console");
            return true;
        }

        let error_code = GetLastError();
        if error_code == ERROR_ACCESS_DENIED {
            // Already attached to a console.
            return true;
        }

        if error_code == ERROR_INVALID_HANDLE {
            return false;
        } else {
            warn!(
                "startup: failed to attach parent console (error {}), Ctrl+C may be unavailable",
                error_code
            );
            return false;
        }
    }
}

#[cfg(target_os = "windows")]
fn read_window_animation_setting() -> Option<ANIMATIONINFO> {
    unsafe {
        let mut info = ANIMATIONINFO {
            cbSize: std::mem::size_of::<ANIMATIONINFO>() as u32,
            iMinAnimate: 1,
        };
        let ok = SystemParametersInfoW(
            SPI_GETANIMATION,
            info.cbSize,
            &mut info as *mut _ as *mut _,
            0,
        );
        if ok == 0 {
            warn!(
                "SPI_GETANIMATION failed (error {})",
                GetLastError()
            );
            None
        } else {
            Some(info)
        }
    }
}

#[cfg(target_os = "windows")]
fn set_window_animation_enabled(enabled: bool) -> bool {
    unsafe {
        let mut info = ANIMATIONINFO {
            cbSize: std::mem::size_of::<ANIMATIONINFO>() as u32,
            iMinAnimate: if enabled { 1 } else { 0 },
        };
        let ok = SystemParametersInfoW(
            SPI_SETANIMATION,
            info.cbSize,
            &mut info as *mut _ as *mut _,
            SPIF_SENDCHANGE,
        );
        if ok == 0 {
            warn!(
                "SPI_SETANIMATION({}) failed (error {})",
                if enabled { "enable" } else { "disable" },
                GetLastError()
            );
            false
        } else {
            true
        }
    }
}

fn parse_cli_args() -> CliArgs {
    let mut capture_on_start = false;
    let mut show_help = false;
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--capture" | "-c" => capture_on_start = true,
            "--help" | "-h" => show_help = true,
            "--version" | "-V" => {
                println!("screenclip {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            other if other.starts_with('-') => {
                eprintln!("unknown flag: {other}\n");
                show_help = true;
            }
            other => {
                eprintln!("unexpected positional argument: {other}\n");
                show_help = true;
            }
        }
    }
    if show_help {
        println!(
            "screenclip {}\n\n\
             USAGE:\n  \
               screenclip [OPTIONS]\n\n\
             OPTIONS:\n  \
               -c, --capture    Skip the global hotkey check and start a capture session\n  \
                               immediately. Useful when the desktop environment (e.g.\n  \
                               GNOME/Wayland) blocks global hotkeys — bind a shortcut to\n  \
                               `screenclip --capture` instead.\n  \
               -h, --help       Show this message and exit\n  \
               -V, --version    Show the version and exit",
            env!("CARGO_PKG_VERSION"),
        );
        std::process::exit(0);
    }
    CliArgs { capture_on_start }
}

struct CliArgs {
    capture_on_start: bool,
}

fn main() {
    env_logger::Builder::from_default_env()
        .filter_module("screenclip", log::LevelFilter::Info)
        .init();

    let CliArgs { capture_on_start } = parse_cli_args();

    let event_loop = EventLoop::<AppEvent>::with_user_event()
        .build()
        .expect("failed to create event loop");
    event_loop.set_control_flow(ControlFlow::Wait);

    let event_proxy = event_loop.create_proxy();
    init_console_ctrl_c_shutdown(event_proxy.clone());

    let mut app = App::new(event_proxy, capture_on_start);
    event_loop.run_app(&mut app).expect("event loop failed");
}

impl ApplicationHandler<AppEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        event_loop.set_control_flow(ControlFlow::Wait);

        if self.initialized {
            return;
        }

        self.text_font_bytes = load_text_font_bytes();
        self.text_fonts = self
            .text_font_bytes
            .iter()
            .filter_map(|(_, bytes)| FontArc::try_from_vec(bytes.clone()).ok())
            .collect();

        match hotkeys::HotkeyRuntime::new(self.event_proxy.clone()) {
            Ok(runtime) => {
                self.hotkeys = Some(runtime);
                hotkeys::log_registration_succeeded();
            }
            Err(err) => hotkeys::log_install_error(&err),
        }

        // `tray-icon` does not auto-initialize GTK on Linux; it panics inside
        // `Menu::new` if gtk is not initialized first. We must call gtk::init
        // before constructing the tray icon, but it is a no-op on other OSes.
        #[cfg(target_os = "linux")]
        gtk::init().expect("failed to initialize GTK for tray icon");

        match tray::TrayRuntime::new(self.event_proxy.clone()) {
            Ok(runtime) => self.tray = Some(runtime),
            Err(err) => tray::log_install_error(err.as_ref()),
        }

        self.initialized = true;

        if self.capture_on_start {
            // Synthesize the same event the global hotkey would emit, then
            // clear the flag so subsequent resumes (e.g. suspend/resume
            // cycles on Wayland) do not re-trigger a capture.
            self.capture_on_start = false;
            let _ = self.event_proxy.send_event(AppEvent::CaptureRequested);
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: AppEvent) {
        match event {
            AppEvent::CaptureRequested => self.begin_capture_session(event_loop),
            AppEvent::QuitRequested => {
                self.end_capture_session();
                event_loop.exit();
            }
            AppEvent::ToggleAutostart => {
                let new_state = !tray::is_autostart_enabled();
                tray::set_autostart(new_state);
            }
        }
    }

    fn window_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if let WindowEvent::KeyboardInput { event, .. } = &event {
            if event.state == ElementState::Pressed
                && matches!(event.logical_key, Key::Named(NamedKey::Escape))
            {
                let handled = self.handle_escape();
                if handled {
                    return;
                }
            }
        }

        let is_redraw_event = matches!(event, WindowEvent::RedrawRequested);
        if let Some(ws) = self.windows.get_mut(&window_id) {
            let _ = ws.egui.state.on_window_event(&ws.window, &event);
        }
        if !is_redraw_event {
            self.needs_redraw = true;
        }

        match event {
            WindowEvent::CloseRequested => self.end_capture_session(),
            WindowEvent::CursorMoved { position, .. } => {
                if let Some(state) = self.windows.get(&window_id) {
                    let gx = state.origin.x + position.x.round() as i32;
                    let gy = state.origin.y + position.y.round() as i32;
                    self.last_cursor_global = Some((gx, gy));

                    if let Some(drag) = &self.text_annotation_drag {
                        if let Some(annotation) = self
                            .text_annotations
                            .iter_mut()
                            .find(|annotation| annotation.id == drag.id)
                        {
                            annotation.global_pos =
                                (gx - drag.pointer_offset.0, gy - drag.pointer_offset.1);
                        }
                    } else if let Some(resize) = self.selection_resize {
                        if let Some(rect) = self.selected_rect {
                            self.selected_rect =
                                Some(selection_geometry::resize_rect(rect, resize.edge, (gx, gy)));
                        }
                    } else if self.drag_start.is_some() {
                        self.drag_current = Some((gx, gy));
                    }
                }
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => match state {
                ElementState::Pressed => {
                    if matches!(self.tool_mode, ToolMode::Text) {
                        if let Some(pointer) = self.last_cursor_global {
                            if let Some(id) = self.focused_text_annotation_id {
                                if annotations::text::border_hit_test(
                                    &self.text_annotations,
                                    &self.text_fonts,
                                    id,
                                    pointer,
                                ) {
                                    if let Some(annotation) = self
                                        .text_annotations
                                        .iter()
                                        .find(|annotation| annotation.id == id)
                                    {
                                        self.text_annotation_drag = Some(TextAnnotationDrag {
                                            id,
                                            pointer_offset: (
                                                pointer.0 - annotation.global_pos.0,
                                                pointer.1 - annotation.global_pos.1,
                                            ),
                                        });
                                    }
                                    return;
                                }
                            }
                        }
                    }

                    let egui_blocks = self
                        .windows
                        .get(&window_id)
                        .map(|s| {
                            s.egui.ctx.wants_pointer_input() || s.egui.ctx.is_pointer_over_area()
                        })
                        .unwrap_or(false);
                    if !egui_blocks {
                        match self.tool_mode {
                            ToolMode::Select => {
                                if let (Some(pointer), Some(rect)) =
                                    (self.last_cursor_global, self.selected_rect)
                                {
                                    if let Some(edge) =
                                        selection_geometry::detect_resize_edge(rect, pointer)
                                    {
                                        self.selection_resize = Some(SelectionResizeDrag { edge });
                                        return;
                                    }
                                }
                                if let Some(p) = self.last_cursor_global {
                                    self.selection_resize = None;
                                    self.drag_start = Some(p);
                                    self.drag_current = Some(p);
                                    self.selected_rect = None;
                                }
                            }
                            ToolMode::Text => {
                                if let Some(p) = self.last_cursor_global {
                                    let id = self.next_text_annotation_id;
                                    self.next_text_annotation_id += 1;
                                    self.text_annotations.push(TextAnnotation::new(
                                        id,
                                        p,
                                        TEXT_LINE_HEIGHT_PIXELS,
                                    ));
                                    self.focused_text_annotation_id = Some(id);
                                    self.pending_text_focus_id = Some(id);
                                }
                            }
                            ToolMode::Rectangle => {
                                if let Some(p) = self.last_cursor_global {
                                    self.selection_resize = None;
                                    self.drag_start = Some(p);
                                    self.drag_current = Some(p);
                                }
                            }
                        }
                    }
                }
                ElementState::Released => {
                    if self.text_annotation_drag.take().is_some() {
                        return;
                    }

                    if self.selection_resize.take().is_some() {
                        return;
                    }

                    match self.tool_mode {
                        ToolMode::Text => {}
                        ToolMode::Rectangle => {
                            if let Some(rect) =
                                current_selection_rect(self.drag_start, self.drag_current)
                            {
                                if rect.2 > 0 && rect.3 > 0 {
                                    self.rectangle_annotations
                                        .push(RectangleAnnotation { global_rect: rect });
                                }
                            }
                            self.drag_start = None;
                            self.drag_current = None;
                        }
                        ToolMode::Select => {
                            if let Some(rect) =
                                current_selection_rect(self.drag_start, self.drag_current)
                            {
                                self.selected_rect = Some(rect);
                            }
                            self.drag_start = None;
                            self.drag_current = None;
                        }
                    }
                }
            },
            WindowEvent::Resized(new_size) => {
                if let Some(state) = self.windows.get_mut(&window_id) {
                    if new_size.width > 0 && new_size.height > 0 {
                        state.size = new_size;
                        state.renderer.resize(new_size.width, new_size.height);
                        if let Err(err) = state.presenter.resize(new_size.width, new_size.height) {
                            warn!("resize presenter failed: {}", err);
                        }

                        if state.screenshot.width() != new_size.width
                            || state.screenshot.height() != new_size.height
                        {
                            if state.source_screenshot.width() == new_size.width
                                && state.source_screenshot.height() == new_size.height
                            {
                                state.screenshot = state.source_screenshot.clone();
                            } else {
                                state.screenshot = screenshots::image::imageops::resize(
                                    &state.source_screenshot,
                                    new_size.width,
                                    new_size.height,
                                    FilterType::CatmullRom,
                                );
                            }
                        }
                    }
                }
            }
            WindowEvent::RedrawRequested => {}
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if !self.initialized {
            event_loop.set_control_flow(ControlFlow::Wait);
            return;
        }

        if self.windows.is_empty() {
            event_loop.set_control_flow(ControlFlow::Wait);
            return;
        }

        let now = Instant::now();
        let caret_redraw_due = self
            .next_caret_redraw
            .is_some_and(|deadline| now >= deadline);

        if !self.needs_redraw && !caret_redraw_due {
            if let Some(deadline) = self.next_caret_redraw {
                event_loop.set_control_flow(ControlFlow::WaitUntil(deadline));
            } else {
                event_loop.set_control_flow(ControlFlow::Wait);
            }
            return;
        }

        self.needs_redraw = false;

        let drag_rect = current_selection_rect(self.drag_start, self.drag_current);
        let active_selection_rect = match self.tool_mode {
            ToolMode::Select => drag_rect.or(self.selected_rect),
            ToolMode::Text | ToolMode::Rectangle => self.selected_rect,
        };
        let active_annotation_rect = match self.tool_mode {
            ToolMode::Rectangle => drag_rect,
            _ => None,
        };
        let show_toolbar = self.drag_start.is_none()
            && self.selection_resize.is_none()
            && self.selected_rect.is_some();

        let mut pending_save = false;
        let mut pending_copy = false;
        let mut pending_exit = false;
        let mut focused_text_annotation_id = None;
        let existing_focused_text_annotation_id = self.focused_text_annotation_id;
        let dragging_text_annotation_id = self.text_annotation_drag.as_ref().map(|drag| drag.id);
        let selection_resize_edge = self.selection_resize.map(|drag| drag.edge);
        for state in self.windows.values_mut() {
            let (action, focused_annotation_id) = render_window_cpu(
                state,
                active_selection_rect,
                active_annotation_rect,
                self.selected_rect,
                self.last_cursor_global,
                show_toolbar,
                self.tool_mode,
                &mut self.text_annotations,
                &self.rectangle_annotations,
                &mut self.pending_text_focus_id,
                &self.text_fonts,
                existing_focused_text_annotation_id,
                dragging_text_annotation_id,
                selection_resize_edge,
            );
            if let Some(id) = focused_annotation_id {
                focused_text_annotation_id = Some(id);
            }
            if action == OverlayAction::Save {
                info!("[about_to_wait] OverlayAction::Save received");
                pending_save = true;
            } else if action == OverlayAction::Copy {
                info!("[about_to_wait] OverlayAction::Copy received");
                pending_copy = true;
            } else if action == OverlayAction::StartTextInsert {
                self.tool_mode = if self.tool_mode == ToolMode::Text {
                    ToolMode::Select
                } else {
                    ToolMode::Text
                };
            } else if action == OverlayAction::StartRectangleInsert {
                self.tool_mode = if self.tool_mode == ToolMode::Rectangle {
                    ToolMode::Select
                } else {
                    ToolMode::Rectangle
                };
            } else if action == OverlayAction::Exit {
                info!("[about_to_wait] OverlayAction::Exit received");
                pending_exit = true;
            }
        }
        if focused_text_annotation_id.is_none() {
            focused_text_annotation_id = dragging_text_annotation_id;
        }
        self.focused_text_annotation_id = focused_text_annotation_id;

        if pending_exit {
            self.end_capture_session();
            return;
        }

        if pending_save {
            info!(
                "[save] pending_save=true selected_rect={:?}",
                self.selected_rect
            );
            if let Some(rect) = self.selected_rect {
                for ws in self.windows.values() {
                    ws.window.set_visible(false);
                }
                info!("[save] windows hidden, opening file dialog");
                let path = rfd::FileDialog::new()
                    .set_file_name("screenshot.png")
                    .add_filter("PNG Image", &["png"])
                    .save_file();
                info!("[save] file dialog returned {:?}", path);
                if let Some(path) = path {
                    match save_selection_to_file(
                        &self.captures,
                        rect,
                        &path,
                        &self.text_annotations,
                        &self.rectangle_annotations,
                        &self.text_fonts,
                    ) {
                        Ok(()) => info!("saved screenshot to {}", path.display()),
                        Err(e) => warn!("failed to save screenshot: {}", e),
                    }
                }
            }
            self.end_capture_session();
            return;
        }

        if pending_copy {
            info!(
                "[copy] pending_copy=true selected_rect={:?}",
                self.selected_rect
            );
            if let Some(rect) = self.selected_rect {
                match compose_selection_image(
                    &self.captures,
                    rect,
                    &self.text_annotations,
                    &self.rectangle_annotations,
                    &self.text_fonts,
                ) {
                    Ok(image) => match copy_image_to_clipboard(image) {
                        Ok(()) => info!("copied screenshot to clipboard"),
                        Err(err) => warn!("failed to copy screenshot to clipboard: {}", err),
                    },
                    Err(err) => warn!("failed to compose screenshot for clipboard: {}", err),
                }
            }
            self.end_capture_session();
            return;
        }

        if matches!(self.tool_mode, ToolMode::Text) {
            let deadline = Instant::now() + TEXT_CARET_BLINK_INTERVAL;
            self.next_caret_redraw = Some(deadline);
            event_loop.set_control_flow(ControlFlow::WaitUntil(deadline));
        } else {
            self.next_caret_redraw = None;
            event_loop.set_control_flow(ControlFlow::Wait);
        }
    }
}

fn current_selection_rect(
    start: Option<(i32, i32)>,
    current: Option<(i32, i32)>,
) -> Option<(i32, i32, i32, i32)> {
    selection_geometry::selection_rect_from_points(start, current)
}

fn save_selection_to_file(
    captures: &[CapturedMonitor],
    rect: (i32, i32, i32, i32),
    path: &PathBuf,
    text_annotations: &[TextAnnotation],
    rectangle_annotations: &[RectangleAnnotation],
    text_fonts: &[FontArc],
) -> Result<(), String> {
    let output = compose_selection_image(
        captures,
        rect,
        text_annotations,
        rectangle_annotations,
        text_fonts,
    )?;

    output
        .save(path)
        .map_err(|err| format!("failed to save image: {err}"))?;

    Ok(())
}

fn compose_selection_image(
    captures: &[CapturedMonitor],
    rect: (i32, i32, i32, i32),
    text_annotations: &[TextAnnotation],
    rectangle_annotations: &[RectangleAnnotation],
    text_fonts: &[FontArc],
) -> Result<RgbaImage, String> {
    let (x, y, w, h) = rect;
    if w <= 0 || h <= 0 {
        return Err("selection size must be positive".to_string());
    }

    let width = u32::try_from(w).map_err(|_| "selection width is out of range".to_string())?;
    let height = u32::try_from(h).map_err(|_| "selection height is out of range".to_string())?;
    let mut output = RgbaImage::from_pixel(width, height, Rgba([0, 0, 0, 255]));

    for capture in captures {
        let capture_x0 = capture.origin.x;
        let capture_y0 = capture.origin.y;
        let capture_x1 = capture_x0 + capture.image.width() as i32;
        let capture_y1 = capture_y0 + capture.image.height() as i32;
        let rect_x1 = x + w;
        let rect_y1 = y + h;

        let intersect_x0 = x.max(capture_x0);
        let intersect_y0 = y.max(capture_y0);
        let intersect_x1 = rect_x1.min(capture_x1);
        let intersect_y1 = rect_y1.min(capture_y1);

        if intersect_x0 >= intersect_x1 || intersect_y0 >= intersect_y1 {
            continue;
        }

        for global_y in intersect_y0..intersect_y1 {
            for global_x in intersect_x0..intersect_x1 {
                let src_x = (global_x - capture_x0) as u32;
                let src_y = (global_y - capture_y0) as u32;
                let dst_x = (global_x - x) as u32;
                let dst_y = (global_y - y) as u32;
                let pixel = *capture.image.get_pixel(src_x, src_y);
                output.put_pixel(dst_x, dst_y, pixel);
            }
        }
    }

    annotations::text::render_to_image(&mut output, rect, text_annotations, text_fonts)?;
    annotations::rectangle::render_to_image(&mut output, rect, rectangle_annotations);

    Ok(output)
}

fn copy_image_to_clipboard(image: RgbaImage) -> Result<(), String> {
    let (width, height) = image.dimensions();
    let bytes = image.into_raw();
    let mut clipboard = Clipboard::new().map_err(|err| format!("clipboard init failed: {err}"))?;
    clipboard
        .set_image(ImageData {
            width: width as usize,
            height: height as usize,
            bytes: Cow::Owned(bytes),
        })
        .map_err(|err| format!("clipboard write failed: {err}"))
}

fn load_text_font_bytes() -> Vec<(String, Vec<u8>)> {
    let mut fonts = Vec::new();

    #[cfg(target_os = "windows")]
    {
        // Keep preview and export text rendering aligned by using the same fallback stack.
        let candidates = [
            ("Segoe UI", r"C:\Windows\Fonts\segoeui.ttf"),
            ("Segoe UI Symbol", r"C:\Windows\Fonts\seguisym.ttf"),
            ("Arial", r"C:\Windows\Fonts\arial.ttf"),
            ("Consolas", r"C:\Windows\Fonts\consola.ttf"),
            ("Microsoft YaHei", r"C:\Windows\Fonts\msyh.ttc"),
        ];
        for (name, path) in candidates {
            if let Ok(bytes) = std::fs::read(path) {
                info!("loaded text font {} from {}", name, path);
                fonts.push((name.to_string(), bytes));
            }
        }
    }

    fonts
}

fn capture_or_fallback(screens: &[Screen], x: i32, y: i32, width: u32, height: u32) -> RgbaImage {
    let matched = screens.iter().find(|s| {
        let info = s.display_info;
        info.x == x && info.y == y && info.width == width && info.height == height
    });

    if let Some(screen) = matched {
        match screen.capture() {
            Ok(image) => {
                info!("captured screenshot at ({}, {}) {}x{}", x, y, width, height);
                return image;
            }
            Err(err) => {
                warn!(
                    "capture failed for monitor at ({}, {}) {}x{}: {}",
                    x, y, width, height, err
                );
            }
        }
    }

    RgbaImage::from_pixel(width.max(1), height.max(1), Rgba([70, 70, 70, 255]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::OnceLock;

    #[cfg(target_os = "windows")]
    use winit::platform::windows::EventLoopBuilderExtWindows;

    #[test]
    fn escape_action_prefers_exiting_text_mode() {
        assert_eq!(
            EscapeAction::for_overlay_state(true, ToolMode::Text),
            Some(EscapeAction::ExitTextMode)
        );
    }

    #[test]
    fn escape_action_exits_rectangle_mode() {
        assert_eq!(
            EscapeAction::for_overlay_state(true, ToolMode::Rectangle),
            Some(EscapeAction::ExitRectangleMode)
        );
    }

    #[test]
    fn escape_action_closes_session_in_select_mode() {
        assert_eq!(
            EscapeAction::for_overlay_state(true, ToolMode::Select),
            Some(EscapeAction::CloseSession)
        );
    }

    #[test]
    fn escape_action_is_ignored_without_active_session() {
        assert_eq!(
            EscapeAction::for_overlay_state(false, ToolMode::Select),
            None
        );
        assert_eq!(EscapeAction::for_overlay_state(false, ToolMode::Text), None);
        assert_eq!(
            EscapeAction::for_overlay_state(false, ToolMode::Rectangle),
            None
        );
    }

    #[test]
    fn exit_text_mode_clears_text_edit_state_only() {
        let mut app = App::new(test_event_proxy(), false);
        app.tool_mode = ToolMode::Text;
        app.selected_rect = Some((1, 2, 3, 4));
        app.pending_text_focus_id = Some(7);
        app.focused_text_annotation_id = Some(7);
        app.text_annotation_drag = Some(TextAnnotationDrag {
            id: 7,
            pointer_offset: (3, 4),
        });
        app.next_caret_redraw = Some(Instant::now());
        app.needs_redraw = false;

        app.exit_text_mode();

        assert_eq!(app.tool_mode, ToolMode::Select);
        assert_eq!(app.pending_text_focus_id, None);
        assert_eq!(app.focused_text_annotation_id, None);
        assert!(app.text_annotation_drag.is_none());
        assert!(app.next_caret_redraw.is_none());
        assert!(app.needs_redraw);
        assert_eq!(app.selected_rect, Some((1, 2, 3, 4)));
    }

    fn test_event_proxy() -> EventLoopProxy<AppEvent> {
        static TEST_EVENT_PROXY: OnceLock<EventLoopProxy<AppEvent>> = OnceLock::new();

        TEST_EVENT_PROXY
            .get_or_init(|| {
                #[cfg(target_os = "windows")]
                let event_loop = EventLoop::<AppEvent>::with_user_event()
                    .with_any_thread(true)
                    .build()
                    .expect("failed to create test event loop");

                #[cfg(not(target_os = "windows"))]
                let event_loop = EventLoop::<AppEvent>::with_user_event()
                    .build()
                    .expect("failed to create test event loop");

                event_loop.create_proxy()
            })
            .clone()
    }
}

fn render_window_cpu(
    state: &mut WindowState,
    active_selection_rect: Option<(i32, i32, i32, i32)>,
    active_annotation_rect: Option<(i32, i32, i32, i32)>,
    selected_rect: Option<(i32, i32, i32, i32)>,
    cursor_global: Option<(i32, i32)>,
    show_toolbar: bool,
    tool_mode: ToolMode,
    text_annotations: &mut [TextAnnotation],
    rectangle_annotations: &[RectangleAnnotation],
    pending_text_focus_id: &mut Option<u64>,
    text_fonts: &[FontArc],
    focused_text_annotation_id: Option<u64>,
    dragging_text_annotation_id: Option<u64>,
    selection_resize_edge: Option<ResizeEdge>,
) -> (OverlayAction, Option<u64>) {
    let pixels_per_point = state.window.scale_factor() as f32;
    let raw_input = state.egui.state.take_egui_input(&state.window);
    state.egui.ctx.begin_pass(raw_input);
    let action = state.egui.ui.draw(
        &state.egui.ctx,
        active_selection_rect,
        cursor_global,
        show_toolbar,
        state.origin,
        (state.size.width, state.size.height),
        pixels_per_point,
    );
    let focused_annotation_id = annotations::text::draw_input_hosts(
        &state.egui.ctx,
        state.origin,
        (state.size.width, state.size.height),
        pixels_per_point,
        text_annotations,
        pending_text_focus_id,
        text_fonts,
    );
    if action != OverlayAction::None {
        info!("[render_window_cpu] ui.draw returned {:?}", action);
    }
    let full_output = state.egui.ctx.end_pass();

    let egui_cursor_icon = full_output.platform_output.cursor_icon;
    state
        .egui
        .state
        .handle_platform_output(&state.window, full_output.platform_output);
    if let Some(cursor_icon) = resolve_cursor_icon(
        egui_cursor_icon,
        tool_mode,
        cursor_global,
        selected_rect,
        text_annotations,
        text_fonts,
        focused_annotation_id.or(focused_text_annotation_id),
        dragging_text_annotation_id,
        selection_resize_edge,
    ) {
        state.window.set_cursor(Cursor::Icon(cursor_icon));
    }

    let paint_jobs = state
        .egui
        .ctx
        .tessellate(full_output.shapes, pixels_per_point);

    state
        .renderer
        .apply_textures_delta(&full_output.textures_delta);
    state.renderer.render(
        &state.screenshot,
        (state.origin.x, state.origin.y),
        active_selection_rect,
        pixels_per_point,
        &paint_jobs,
        |renderer| {
            let visual_focused_annotation_id = focused_annotation_id
                .or(dragging_text_annotation_id)
                .or(focused_text_annotation_id);
            annotations::text::draw_preview(
                renderer.frame_mut(),
                (state.size.width, state.size.height),
                state.origin,
                text_annotations,
                text_fonts,
                visual_focused_annotation_id,
            );
            annotations::rectangle::draw_preview(
                renderer.frame_mut(),
                (state.size.width, state.size.height),
                state.origin,
                rectangle_annotations,
                active_annotation_rect,
            );
        },
    );

    if let Err(err) =
        state
            .presenter
            .present(state.renderer.frame(), state.size.width, state.size.height)
    {
        warn!("software present failed: {}", err);
    }

    if state.needs_show {
        state.needs_show = false;
        state.window.set_visible(true);
        state.window.focus_window();
    }

    (action, focused_annotation_id)
}

fn resolve_cursor_icon(
    egui_cursor_icon: egui::CursorIcon,
    tool_mode: ToolMode,
    cursor_global: Option<(i32, i32)>,
    selected_rect: Option<(i32, i32, i32, i32)>,
    text_annotations: &[TextAnnotation],
    text_fonts: &[FontArc],
    focused_text_annotation_id: Option<u64>,
    dragging_text_annotation_id: Option<u64>,
    selection_resize_edge: Option<ResizeEdge>,
) -> Option<CursorIcon> {
    if dragging_text_annotation_id.is_some() {
        return Some(CursorIcon::Grabbing);
    }

    if let Some(edge) = selection_resize_edge {
        return Some(cursor_icon_for_resize_edge(edge));
    }

    let in_text_mode = matches!(tool_mode, ToolMode::Text);
    let in_select_mode = matches!(tool_mode, ToolMode::Select);

    if let Some(point) = cursor_global {
        if in_text_mode {
            if let Some(TextAnnotationCursorHit::Border) = annotations::text::cursor_hit_test(
                text_annotations,
                text_fonts,
                focused_text_annotation_id,
                point,
            ) {
                return Some(CursorIcon::Move);
            }
        }

        if in_select_mode {
            if let Some(rect) = selected_rect {
                if let Some(edge) = selection_geometry::detect_resize_edge(rect, point) {
                    return Some(cursor_icon_for_resize_edge(edge));
                }
            }
        }
    }

    if egui_cursor_icon != egui::CursorIcon::Default {
        return translate_egui_cursor_icon(egui_cursor_icon);
    }

    if in_text_mode {
        if let Some(point) = cursor_global {
            match annotations::text::cursor_hit_test(
                text_annotations,
                text_fonts,
                focused_text_annotation_id,
                point,
            ) {
                Some(TextAnnotationCursorHit::Border) => return Some(CursorIcon::Grab),
                Some(TextAnnotationCursorHit::Text) => return Some(CursorIcon::Text),
                None => {}
            }
        }

        return Some(CursorIcon::Text);
    }

    Some(CursorIcon::Crosshair)
}

fn cursor_icon_for_resize_edge(edge: ResizeEdge) -> CursorIcon {
    match edge {
        ResizeEdge::Top => CursorIcon::NResize,
        ResizeEdge::Bottom => CursorIcon::SResize,
        ResizeEdge::Left => CursorIcon::WResize,
        ResizeEdge::Right => CursorIcon::EResize,
    }
}

fn translate_egui_cursor_icon(cursor_icon: egui::CursorIcon) -> Option<CursorIcon> {
    match cursor_icon {
        egui::CursorIcon::None => None,
        egui::CursorIcon::Alias => Some(CursorIcon::Alias),
        egui::CursorIcon::AllScroll => Some(CursorIcon::AllScroll),
        egui::CursorIcon::Cell => Some(CursorIcon::Cell),
        egui::CursorIcon::ContextMenu => Some(CursorIcon::ContextMenu),
        egui::CursorIcon::Copy => Some(CursorIcon::Copy),
        egui::CursorIcon::Crosshair => Some(CursorIcon::Crosshair),
        egui::CursorIcon::Default => Some(CursorIcon::Default),
        egui::CursorIcon::Grab => Some(CursorIcon::Grab),
        egui::CursorIcon::Grabbing => Some(CursorIcon::Grabbing),
        egui::CursorIcon::Help => Some(CursorIcon::Help),
        egui::CursorIcon::Move => Some(CursorIcon::Move),
        egui::CursorIcon::NoDrop => Some(CursorIcon::NoDrop),
        egui::CursorIcon::NotAllowed => Some(CursorIcon::NotAllowed),
        egui::CursorIcon::PointingHand => Some(CursorIcon::Pointer),
        egui::CursorIcon::Progress => Some(CursorIcon::Progress),
        egui::CursorIcon::ResizeHorizontal => Some(CursorIcon::EwResize),
        egui::CursorIcon::ResizeNeSw => Some(CursorIcon::NeswResize),
        egui::CursorIcon::ResizeNwSe => Some(CursorIcon::NwseResize),
        egui::CursorIcon::ResizeVertical => Some(CursorIcon::NsResize),
        egui::CursorIcon::ResizeEast => Some(CursorIcon::EResize),
        egui::CursorIcon::ResizeSouthEast => Some(CursorIcon::SeResize),
        egui::CursorIcon::ResizeSouth => Some(CursorIcon::SResize),
        egui::CursorIcon::ResizeSouthWest => Some(CursorIcon::SwResize),
        egui::CursorIcon::ResizeWest => Some(CursorIcon::WResize),
        egui::CursorIcon::ResizeNorthWest => Some(CursorIcon::NwResize),
        egui::CursorIcon::ResizeNorth => Some(CursorIcon::NResize),
        egui::CursorIcon::ResizeNorthEast => Some(CursorIcon::NeResize),
        egui::CursorIcon::ResizeColumn => Some(CursorIcon::ColResize),
        egui::CursorIcon::ResizeRow => Some(CursorIcon::RowResize),
        egui::CursorIcon::Text => Some(CursorIcon::Text),
        egui::CursorIcon::VerticalText => Some(CursorIcon::VerticalText),
        egui::CursorIcon::Wait => Some(CursorIcon::Wait),
        egui::CursorIcon::ZoomIn => Some(CursorIcon::ZoomIn),
        egui::CursorIcon::ZoomOut => Some(CursorIcon::ZoomOut),
    }
}
