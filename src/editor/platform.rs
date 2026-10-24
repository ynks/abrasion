use slint::platform::software_renderer::{
    MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType, TargetPixel,
};
use slint::platform::{Platform, WindowAdapter};
use slint::PlatformError;
use std::cell::RefCell;
use std::rc::Rc;

thread_local! {
    static NEXT: RefCell<Option<Rc<MinimalSoftwareWindow>>> = const { RefCell::new(None) };
}

struct EditorPlatform;

impl Platform for EditorPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        NEXT.with(|w| w.borrow_mut().take())
            .map(|w| w as Rc<dyn WindowAdapter>)
            .ok_or_else(|| PlatformError::Other("no window".into()))
    }
}

// next AppWindow on this thread picks this up
pub fn next_window() -> Rc<MinimalSoftwareWindow> {
    // only works once per thread
    let _ = slint::platform::set_platform(Box::new(EditorPlatform));
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    NEXT.with(|w| *w.borrow_mut() = Some(window.clone()));
    window
}

// softbuffer wants 0RGB
#[derive(Clone, Copy)]
#[repr(transparent)]
pub struct Pixel(u32);

impl TargetPixel for Pixel {
    fn blend(&mut self, c: PremultipliedRgbaColor) {
        let a = (255 - c.alpha) as u32;
        let r = (((self.0 >> 16) & 0xff) * a / 255 + c.red as u32).min(255);
        let g = (((self.0 >> 8) & 0xff) * a / 255 + c.green as u32).min(255);
        let b = ((self.0 & 0xff) * a / 255 + c.blue as u32).min(255);
        self.0 = (r << 16) | (g << 8) | b;
    }

    fn from_rgb(r: u8, g: u8, b: u8) -> Self {
        Self(((r as u32) << 16) | ((g as u32) << 8) | b as u32)
    }
}

pub fn as_pixels(buffer: &mut [u32]) -> &mut [Pixel] {
    unsafe { std::slice::from_raw_parts_mut(buffer.as_mut_ptr().cast(), buffer.len()) }
}
