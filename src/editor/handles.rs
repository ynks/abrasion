// baseview and softbuffer use different raw window handle versions

use nih_plug::prelude::ParentWindowHandle;
use raw_window_handle::{
    HasRawDisplayHandle, HasRawWindowHandle, RawDisplayHandle, RawWindowHandle,
};
use raw_window_handle_06 as rwh;
use std::num::{NonZeroIsize, NonZeroU32};
use std::ptr::NonNull;

pub struct ParentHandle(pub ParentWindowHandle);

unsafe impl HasRawWindowHandle for ParentHandle {
    fn raw_window_handle(&self) -> RawWindowHandle {
        match self.0 {
            ParentWindowHandle::X11Window(window) => {
                let mut h = raw_window_handle::XcbWindowHandle::empty();
                h.window = window;
                RawWindowHandle::Xcb(h)
            }
            ParentWindowHandle::AppKitNsView(ns_view) => {
                let mut h = raw_window_handle::AppKitWindowHandle::empty();
                h.ns_view = ns_view;
                RawWindowHandle::AppKit(h)
            }
            ParentWindowHandle::Win32Hwnd(hwnd) => {
                let mut h = raw_window_handle::Win32WindowHandle::empty();
                h.hwnd = hwnd;
                RawWindowHandle::Win32(h)
            }
        }
    }
}

#[derive(Clone)]
pub struct SurfaceTarget {
    display: rwh::RawDisplayHandle,
    window: rwh::RawWindowHandle,
}

impl rwh::HasDisplayHandle for SurfaceTarget {
    fn display_handle(&self) -> Result<rwh::DisplayHandle<'_>, rwh::HandleError> {
        Ok(unsafe { rwh::DisplayHandle::borrow_raw(self.display) })
    }
}

impl rwh::HasWindowHandle for SurfaceTarget {
    fn window_handle(&self) -> Result<rwh::WindowHandle<'_>, rwh::HandleError> {
        Ok(unsafe { rwh::WindowHandle::borrow_raw(self.window) })
    }
}

pub fn surface_target(window: &baseview::Window<'_>) -> SurfaceTarget {
    let display = match window.raw_display_handle() {
        RawDisplayHandle::AppKit(_) => {
            rwh::RawDisplayHandle::AppKit(rwh::AppKitDisplayHandle::new())
        }
        RawDisplayHandle::Xlib(h) => rwh::RawDisplayHandle::Xlib(rwh::XlibDisplayHandle::new(
            NonNull::new(h.display),
            h.screen,
        )),
        RawDisplayHandle::Xcb(h) => rwh::RawDisplayHandle::Xcb(rwh::XcbDisplayHandle::new(
            NonNull::new(h.connection),
            h.screen,
        )),
        _ => rwh::RawDisplayHandle::Windows(rwh::WindowsDisplayHandle::new()),
    };

    let window = match window.raw_window_handle() {
        RawWindowHandle::AppKit(h) => rwh::RawWindowHandle::AppKit(rwh::AppKitWindowHandle::new(
            NonNull::new(h.ns_view).unwrap(),
        )),
        RawWindowHandle::Xlib(h) => {
            rwh::RawWindowHandle::Xlib(rwh::XlibWindowHandle::new(h.window))
        }
        RawWindowHandle::Xcb(h) => rwh::RawWindowHandle::Xcb(rwh::XcbWindowHandle::new(
            NonZeroU32::new(h.window).unwrap(),
        )),
        RawWindowHandle::Win32(h) => {
            let mut handle =
                rwh::Win32WindowHandle::new(NonZeroIsize::new(h.hwnd as isize).unwrap());
            handle.hinstance = NonZeroIsize::new(h.hinstance as isize);
            rwh::RawWindowHandle::Win32(handle)
        }
        other => panic!("unsupported window {other:?}"),
    };

    SurfaceTarget { display, window }
}
