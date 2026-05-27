#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod cpu_present;
mod cpu_renderer;
mod desktop_geometry;
mod hotkeys;
mod overlay_ui;
mod overlay_ui_utils;
mod selection_geometry;
mod text_annotations;
mod tray;

use std::{collections::HashMap, path::PathBuf, sync::Arc, time::Instant};

use ab_glyph::FontArc;
use arboard::{Clipboard, ImageData};
use cpu_present::CpuPresenter;
use cpu_renderer::CpuRenderer;
use log::{info, warn};
use overlay_ui::{OverlayAction, OverlayUi};
use screenshots::image::{imageops::FilterType, Rgba, RgbaImage};
use screenshots::Screen;
use selection_geometry::ResizeEdge;
use text_annotations::{
    TextAnnotation, TextAnnotationCursorHit, TextAnnotationDrag, TEXT_CARET_BLINK_INTERVAL,
    TEXT_LINE_HEIGHT_PIXELS,
};
use winit::{
    application::ApplicationHandler,
    dpi::{PhysicalPosition, PhysicalSize, Position, Size},
    event::{ElementState, MouseButton, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy},
    keyboard::{Key, NamedKey},
    monitor::MonitorHandle,
    window::{Cursor, CursorIcon, Fullscreen, Window, WindowAttributes, WindowId, WindowLevel},
};

use std::borrow::Cow;

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
    text_insert_mode: bool,
    text_annotations: Vec<TextAnnotation>,
    next_text_annotation_id: u64,
    pending_text_focus_id: Option<u64>,
    text_font_bytes: Vec<(String, Vec<u8>)>,
    text_fonts: Vec<FontArc>,
    needs_redraw: bool,
    next_caret_redraw: Option<Instant>,
    focused_text_annotation_id: Option<u64>,
    text_annotation_drag: Option<TextAnnotationDrag>,
}

impl App {
    fn new(event_proxy: EventLoopProxy<AppEvent>) -> Self {
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
            text_insert_mode: false,
            text_annotations: Vec::new(),
            next_text_annotation_id: 0,
            pending_text_focus_id: None,
            text_font_bytes: Vec::new(),
            text_fonts: Vec::new(),
            needs_redraw: false,
            next_caret_redraw: None,
            focused_text_annotation_id: None,
            text_annotation_drag: None,
        }
    }

    fn begin_capture_session(&mut self, event_loop: &ActiveEventLoop) {
        if !self.windows.is_empty() {
            info!("screenshot hotkey ignored because a capture session is already active");
            return;
        }

        self.reset_capture_state();

        let monitors: Vec<MonitorHandle> = event_loop.available_monitors().collect();
        let screens = Screen::all().unwrap_or_default();
        info!("detected {} monitors", monitors.len());

        for (idx, monitor) in monitors.into_iter().enumerate() {
            let size = monitor.size();
            let pos = monitor.position();
            let monitor_for_window = monitor.clone();

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
            window.set_fullscreen(Some(Fullscreen::Borderless(Some(monitor_for_window))));
            window.set_outer_position(PhysicalPosition::new(pos.x, pos.y));
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
    }

    fn reset_capture_state(&mut self) {
        self.windows.clear();
        self.captures.clear();
        self.last_cursor_global = None;
        self.drag_start = None;
        self.drag_current = None;
        self.selected_rect = None;
        self.selection_resize = None;
        self.text_insert_mode = false;
        self.text_annotations.clear();
        self.next_text_annotation_id = 0;
        self.pending_text_focus_id = None;
        self.needs_redraw = false;
        self.next_caret_redraw = None;
        self.focused_text_annotation_id = None;
        self.text_annotation_drag = None;
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

fn main() {
    env_logger::Builder::from_default_env()
        .filter_module("screenclip", log::LevelFilter::Info)
        .init();

    let event_loop = EventLoop::<AppEvent>::with_user_event()
        .build()
        .expect("failed to create event loop");
    event_loop.set_control_flow(ControlFlow::Wait);

    let mut app = App::new(event_loop.create_proxy());
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
            Ok(runtime) => self.hotkeys = Some(runtime),
            Err(err) => hotkeys::log_install_error(&err),
        }

        match tray::TrayRuntime::new(self.event_proxy.clone()) {
            Ok(runtime) => self.tray = Some(runtime),
            Err(err) => tray::log_install_error(err.as_ref()),
        }

        self.initialized = true;
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
        let is_redraw_event = matches!(event, WindowEvent::RedrawRequested);
        if let Some(ws) = self.windows.get_mut(&window_id) {
            let _ = ws.egui.state.on_window_event(&ws.window, &event);
        }
        if !is_redraw_event {
            self.needs_redraw = true;
        }

        match event {
            WindowEvent::CloseRequested => self.end_capture_session(),
            WindowEvent::KeyboardInput { event, .. } => {
                if event.state == ElementState::Pressed
                    && matches!(event.logical_key, Key::Named(NamedKey::Escape))
                {
                    self.end_capture_session();
                }
            }
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
                    if self.text_insert_mode {
                        if let Some(pointer) = self.last_cursor_global {
                            if let Some(id) = self.focused_text_annotation_id {
                                if text_annotations::border_hit_test(
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
                        if !self.text_insert_mode {
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
                        }

                        if self.text_insert_mode {
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
                        } else if let Some(p) = self.last_cursor_global {
                            self.selection_resize = None;
                            self.drag_start = Some(p);
                            self.drag_current = Some(p);
                            self.selected_rect = None;
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

                    if self.text_insert_mode {
                        return;
                    }
                    if let Some(rect) = current_selection_rect(self.drag_start, self.drag_current) {
                        self.selected_rect = Some(rect);
                    }
                    self.drag_start = None;
                    self.drag_current = None;
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

        let active_rect =
            current_selection_rect(self.drag_start, self.drag_current).or(self.selected_rect);
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
                active_rect,
                self.selected_rect,
                self.last_cursor_global,
                show_toolbar,
                self.text_insert_mode,
                &mut self.text_annotations,
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
                self.text_insert_mode = true;
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

        if self.text_insert_mode {
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
    text_fonts: &[FontArc],
) -> Result<(), String> {
    let output = compose_selection_image(captures, rect, text_annotations, text_fonts)?;

    output
        .save(path)
        .map_err(|err| format!("failed to save image: {err}"))?;

    Ok(())
}

fn compose_selection_image(
    captures: &[CapturedMonitor],
    rect: (i32, i32, i32, i32),
    text_annotations: &[TextAnnotation],
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

    text_annotations::render_to_image(&mut output, rect, text_annotations, text_fonts)?;

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

fn render_window_cpu(
    state: &mut WindowState,
    active_rect: Option<(i32, i32, i32, i32)>,
    selected_rect: Option<(i32, i32, i32, i32)>,
    cursor_global: Option<(i32, i32)>,
    show_toolbar: bool,
    text_insert_mode: bool,
    text_annotations: &mut [TextAnnotation],
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
        active_rect,
        cursor_global,
        show_toolbar,
        state.origin,
        (state.size.width, state.size.height),
        pixels_per_point,
    );
    let focused_annotation_id = text_annotations::draw_input_hosts(
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
        text_insert_mode,
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
        active_rect,
        pixels_per_point,
        &paint_jobs,
        |renderer| {
            let visual_focused_annotation_id = focused_annotation_id
                .or(dragging_text_annotation_id)
                .or(focused_text_annotation_id);
            text_annotations::draw_preview(
                renderer.frame_mut(),
                (state.size.width, state.size.height),
                state.origin,
                text_annotations,
                text_fonts,
                visual_focused_annotation_id,
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
    }

    (action, focused_annotation_id)
}

fn resolve_cursor_icon(
    egui_cursor_icon: egui::CursorIcon,
    text_insert_mode: bool,
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

    if let Some(point) = cursor_global {
        if text_insert_mode {
            if let Some(TextAnnotationCursorHit::Border) = text_annotations::cursor_hit_test(
                text_annotations,
                text_fonts,
                focused_text_annotation_id,
                point,
            ) {
                return Some(CursorIcon::Move);
            }
        }

        if !text_insert_mode {
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

    if text_insert_mode {
        if let Some(point) = cursor_global {
            match text_annotations::cursor_hit_test(
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
