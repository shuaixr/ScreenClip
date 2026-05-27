use std::num::NonZeroU32;
use std::sync::Arc;

use softbuffer::{Context, Surface};
use winit::window::Window;

pub struct CpuPresenter {
    _context: Context<Arc<Window>>,
    surface: Surface<Arc<Window>, Arc<Window>>,
    width: u32,
    height: u32,
}

impl CpuPresenter {
    pub fn new(window: Arc<Window>, width: u32, height: u32) -> Result<Self, String> {
        let context = Context::new(window.clone())
            .map_err(|err| format!("failed to create softbuffer context: {err}"))?;
        let surface = Surface::new(&context, window)
            .map_err(|err| format!("failed to create softbuffer surface: {err}"))?;

        let mut presenter = Self {
            _context: context,
            surface,
            width: 0,
            height: 0,
        };
        presenter.resize(width, height)?;
        Ok(presenter)
    }

    pub fn resize(&mut self, width: u32, height: u32) -> Result<(), String> {
        let width = width.max(1);
        let height = height.max(1);
        if self.width == width && self.height == height {
            return Ok(());
        }

        self.surface
            .resize(
                NonZeroU32::new(width).expect("width is non-zero"),
                NonZeroU32::new(height).expect("height is non-zero"),
            )
            .map_err(|err| format!("failed to resize softbuffer surface: {err}"))?;
        self.width = width;
        self.height = height;
        Ok(())
    }

    pub fn present(&mut self, frame: &[u32], width: u32, height: u32) -> Result<(), String> {
        self.resize(width, height)?;

        let mut buffer = self
            .surface
            .buffer_mut()
            .map_err(|err| format!("failed to lock softbuffer surface: {err}"))?;

        if buffer.len() != frame.len() {
            return Err(format!(
                "frame size mismatch: buffer={} frame={}",
                buffer.len(),
                frame.len()
            ));
        }

        buffer.copy_from_slice(frame);
        buffer
            .present()
            .map_err(|err| format!("failed to present softbuffer surface: {err}"))
    }
}
