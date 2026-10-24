use baseview::{
    Event, EventStatus, MouseButton, MouseEvent, ScrollDelta, WindowHandle, WindowOpenOptions,
    WindowScalePolicy,
};
use crossbeam::atomic::AtomicCell;
use keyboard_types::Modifiers;
use nih_plug::params::internals::ParamPtr;
use nih_plug::params::persist::PersistentField;
use nih_plug::prelude::{Editor, GuiContext, ParentWindowHandle};
use serde::{Deserialize, Serialize};
use slint::platform::software_renderer::MinimalSoftwareWindow;
use slint::platform::{Key, PointerEventButton, WindowAdapter, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, Model, ModelRc, PhysicalSize, SharedString, VecModel};
use std::num::NonZeroU32;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

mod analyzer;
mod handles;
mod platform;
mod strata;

use crate::params::{pos_to_freq, AbrParams, MAX_FREQ, MIN_FREQ};
use crate::shared::{Live, ScopeFrame};
use analyzer::Analyzer;
use handles::{surface_target, ParentHandle, SurfaceTarget};
use strata::{StrataRenderer, ViewParams, VIEW_HEIGHT, VIEW_WIDTH};

slint::include_modules!();

pub const DEFAULT_SIZE: (u32, u32) = (560, 420);

// indices into ui_order
const FREQ: usize = 0;
const AMOUNT: usize = 1;
const WIDTH: usize = 3;

const FRAME_TIME: Duration = Duration::from_millis(30);
// keep animating this long after the last input frame
const INPUT_HOLD: Duration = Duration::from_millis(1500);

type Frames = Arc<Mutex<triple_buffer::Output<ScopeFrame>>>;

#[derive(Debug, Serialize, Deserialize)]
pub struct EditorState {
    #[serde(with = "nih_plug::params::persist::serialize_atomic_cell")]
    size: AtomicCell<(u32, u32)>,
    #[serde(skip)]
    open: AtomicBool,
}

impl EditorState {
    pub fn from_size(size: (u32, u32)) -> Arc<Self> {
        Arc::new(Self {
            size: AtomicCell::new(size),
            open: AtomicBool::new(false),
        })
    }

    pub fn is_open(&self) -> bool {
        self.open.load(Ordering::Acquire)
    }
}

impl<'a> PersistentField<'a, EditorState> for Arc<EditorState> {
    fn set(&self, new_value: EditorState) {
        self.size.store(new_value.size.load());
    }

    fn map<F, R>(&self, f: F) -> R
    where
        F: Fn(&EditorState) -> R,
    {
        f(self)
    }
}

pub fn create(params: Arc<AbrParams>, frames: Frames, live: Arc<Live>) -> Option<Box<dyn Editor>> {
    Some(Box::new(AbrEditor {
        params,
        frames,
        live,
        #[cfg(target_os = "macos")]
        scale: AtomicCell::new(None),
        #[cfg(not(target_os = "macos"))]
        scale: AtomicCell::new(Some(1.0)),
    }))
}

struct AbrEditor {
    params: Arc<AbrParams>,
    frames: Frames,
    live: Arc<Live>,
    scale: AtomicCell<Option<f32>>,
}

impl Editor for AbrEditor {
    fn spawn(
        &self,
        parent: ParentWindowHandle,
        context: Arc<dyn GuiContext>,
    ) -> Box<dyn std::any::Any + Send> {
        let (width, height) = DEFAULT_SIZE;
        let scale = self.scale.load();
        let params = self.params.clone();
        let frames = self.frames.clone();
        let live = self.live.clone();

        let window = baseview::Window::open_parented(
            &ParentHandle(parent),
            WindowOpenOptions {
                title: String::from("Abrasion"),
                size: baseview::Size::new(width as f64, height as f64),
                scale: scale
                    .map(|s| WindowScalePolicy::ScaleFactor(s as f64))
                    .unwrap_or(WindowScalePolicy::SystemScaleFactor),
            },
            move |window: &mut baseview::Window<'_>| {
                EditorWindow::new(window, context, params, frames, live, scale.unwrap_or(1.0))
            },
        );

        self.params.editor_state.open.store(true, Ordering::Release);
        Box::new(EditorHandle {
            state: self.params.editor_state.clone(),
            window,
        })
    }

    fn size(&self) -> (u32, u32) {
        DEFAULT_SIZE
    }

    fn set_scale_factor(&self, factor: f32) -> bool {
        if self.params.editor_state.is_open() {
            return false;
        }
        self.scale.store(Some(factor));
        true
    }

    // values get polled every frame instead
    fn param_value_changed(&self, _id: &str, _normalized_value: f32) {}
    fn param_modulation_changed(&self, _id: &str, _modulation_offset: f32) {}
    fn param_values_changed(&self) {}
}

struct EditorHandle {
    state: Arc<EditorState>,
    window: WindowHandle,
}

unsafe impl Send for EditorHandle {}

impl Drop for EditorHandle {
    fn drop(&mut self) {
        self.state.open.store(false, Ordering::Release);
        self.window.close();
    }
}

#[derive(Clone)]
struct Setter {
    ctx: Arc<dyn GuiContext>,
    ptrs: Rc<Vec<ParamPtr>>,
}

impl Setter {
    fn begin(&self, i: usize) {
        if let Some(&p) = self.ptrs.get(i) {
            unsafe { self.ctx.raw_begin_set_parameter(p) };
        }
    }

    fn set(&self, i: usize, value: f32) {
        if let Some(&p) = self.ptrs.get(i) {
            unsafe {
                self.ctx
                    .raw_set_parameter_normalized(p, value.clamp(0.0, 1.0))
            };
        }
    }

    fn set_plain(&self, i: usize, plain: f32) {
        if let Some(&p) = self.ptrs.get(i) {
            self.set(i, unsafe { p.preview_normalized(plain) });
        }
    }

    fn end(&self, i: usize) {
        if let Some(&p) = self.ptrs.get(i) {
            unsafe { self.ctx.raw_end_set_parameter(p) };
        }
    }

    fn reset(&self, i: usize) {
        if let Some(&p) = self.ptrs.get(i) {
            self.begin(i);
            self.set(i, unsafe { p.default_normalized_value() });
            self.end(i);
        }
    }
}

struct EditorWindow {
    params: Arc<AbrParams>,
    frames: Frames,
    live: Arc<Live>,

    _context: softbuffer::Context<SurfaceTarget>,
    surface: softbuffer::Surface<SurfaceTarget, SurfaceTarget>,
    size: (u32, u32),
    scale: f32,

    window: Rc<MinimalSoftwareWindow>,
    ui: AppWindow,

    ptrs: Rc<Vec<ParamPtr>>,
    param_model: Rc<VecModel<ParamState>>,
    last_values: Vec<f32>,

    analyzer: Analyzer,
    renderer: StrataRenderer,
    last_render: Instant,
    last_input: Instant,
    last_view: Option<ViewParams>,
    dirty: bool,
    idle: u32,

    cursor: LogicalPosition,
    modifiers: Modifiers,
}

impl EditorWindow {
    fn new(
        bv_window: &mut baseview::Window<'_>,
        ctx: Arc<dyn GuiContext>,
        params: Arc<AbrParams>,
        frames: Frames,
        live: Arc<Live>,
        scale: f32,
    ) -> Self {
        let size = (
            (DEFAULT_SIZE.0 as f32 * scale).round() as u32,
            (DEFAULT_SIZE.1 as f32 * scale).round() as u32,
        );

        let target = surface_target(bv_window);
        let context = softbuffer::Context::new(target.clone()).expect("softbuffer context");
        let mut surface = softbuffer::Surface::new(&context, target).expect("softbuffer surface");
        surface
            .resize(
                NonZeroU32::new(size.0.max(1)).unwrap(),
                NonZeroU32::new(size.1.max(1)).unwrap(),
            )
            .expect("softbuffer resize");

        let window = platform::next_window();
        let ui = AppWindow::new().expect("slint ui");
        window.dispatch_event(WindowEvent::ScaleFactorChanged {
            scale_factor: scale,
        });
        window.set_size(PhysicalSize::new(size.0, size.1));
        ui.show().expect("slint show");

        let ptrs = Rc::new(params.ui_order());
        let param_model = Rc::new(VecModel::from(vec![ParamState::default(); ptrs.len()]));

        let bridge = ui.global::<Bridge>();
        bridge.set_params(ModelRc::from(param_model.clone()));
        bind_callbacks(
            &bridge,
            Setter {
                ctx,
                ptrs: ptrs.clone(),
            },
        );

        let now = Instant::now();
        let mut this = Self {
            params,
            frames,
            live,
            _context: context,
            surface,
            size,
            scale,
            window,
            ui,
            last_values: vec![f32::NAN; ptrs.len()],
            ptrs,
            param_model,
            analyzer: Analyzer::new(),
            renderer: StrataRenderer::new(),
            last_render: now,
            last_input: now,
            last_view: None,
            dirty: true,
            idle: 0,
            cursor: LogicalPosition::new(0.0, 0.0),
            modifiers: Modifiers::empty(),
        };
        this.sync_params();
        this
    }

    fn sync_params(&mut self) {
        for (i, &ptr) in self.ptrs.iter().enumerate() {
            let value = unsafe { ptr.modulated_normalized_value() };
            if value == self.last_values[i] {
                continue;
            }
            self.last_values[i] = value;
            let state = unsafe {
                ParamState {
                    value,
                    plain: ptr.modulated_plain_value(),
                    text: SharedString::from(ptr.normalized_value_to_string(value, true)),
                }
            };
            self.param_model.set_row_data(i, state);
        }
    }

    fn view_params(&self) -> ViewParams {
        let p = &self.params;
        let freq = p.freq.value();
        let drift = p.drift.value();
        ViewParams {
            // the audio thread only knows the drifted value
            f_eff: if drift > 0.0 { self.live.f_eff() } else { freq },
            amount: p.amount.value(),
            blend: p.blend.value(),
            width: p.width.value(),
            stereo: p.stereo.value(),
            shape: p.shape.value(),
        }
    }

    fn update_display(&mut self) {
        if let Ok(mut frames) = self.frames.lock() {
            if frames.update() {
                self.analyzer.feed(frames.output_buffer());
                self.last_input = Instant::now();
            }
        }

        let view = self.view_params();
        if self.last_view != Some(view) {
            self.last_view = Some(view);
            self.dirty = true;
        }

        let now = Instant::now();
        let elapsed = now - self.last_render;
        let moving = view.amount > 0.0 || now - self.last_input < INPUT_HOLD;
        if !(self.dirty || moving && elapsed >= FRAME_TIME) {
            return;
        }
        self.dirty = false;
        self.last_render = now;

        self.renderer
            .tick(self.analyzer.points(), elapsed.as_secs_f32().min(0.1));
        let w = (VIEW_WIDTH * self.scale).round() as usize;
        let h = (VIEW_HEIGHT * self.scale).round() as usize;
        let img = self.renderer.render(w, h, &view);
        self.ui
            .global::<Bridge>()
            .set_display(slint::Image::from_rgb8(img));
    }

    // slint only knows about modifiers through key events
    fn sync_modifiers(&mut self, modifiers: Modifiers) {
        if modifiers == self.modifiers {
            return;
        }
        for (flag, key) in [
            (Modifiers::SHIFT, Key::Shift),
            (Modifiers::CONTROL, Key::Control),
            (Modifiers::ALT, Key::Alt),
            (Modifiers::META, Key::Meta),
        ] {
            let down = modifiers.contains(flag);
            if self.modifiers.contains(flag) != down {
                let text = SharedString::from(key);
                self.window.dispatch_event(if down {
                    WindowEvent::KeyPressed { text }
                } else {
                    WindowEvent::KeyReleased { text }
                });
            }
        }
        self.modifiers = modifiers;
    }
}

fn bind_callbacks(bridge: &Bridge, setter: Setter) {
    let s = setter.clone();
    bridge.on_begin(move |i| s.begin(i as usize));
    let s = setter.clone();
    bridge.on_set(move |i, v| s.set(i as usize, v));
    let s = setter.clone();
    bridge.on_end(move |i| s.end(i as usize));
    let s = setter.clone();
    bridge.on_reset(move |i| s.reset(i as usize));
    let s = setter.clone();
    bridge.on_set_plain_in_gesture(move |i, v| s.set_plain(i as usize, v));

    let s = setter;
    bridge.on_drag_xy(move |x, y, alt| {
        s.set_plain(FREQ, pos_to_freq(x).clamp(MIN_FREQ, MAX_FREQ));
        s.set(if alt { WIDTH } else { AMOUNT }, y);
    });
}

fn map_button(button: MouseButton) -> PointerEventButton {
    match button {
        MouseButton::Left => PointerEventButton::Left,
        MouseButton::Right => PointerEventButton::Right,
        MouseButton::Middle => PointerEventButton::Middle,
        _ => PointerEventButton::Other,
    }
}

impl baseview::WindowHandler for EditorWindow {
    fn on_frame(&mut self, _window: &mut baseview::Window) {
        slint::platform::update_timers_and_animations();
        self.sync_params();
        self.update_display();

        // redraw every now and then in case the window got covered
        self.idle += 1;
        if self.idle >= 30 {
            self.window.request_redraw();
        }

        let (w, h) = self.size;
        let surface = &mut self.surface;
        let drawn = self.window.draw_if_needed(|renderer| {
            if let Ok(mut buffer) = surface.buffer_mut() {
                if buffer.len() == (w * h) as usize {
                    renderer.render(platform::as_pixels(&mut buffer), w as usize);
                    let _ = buffer.present();
                }
            }
        });
        if drawn {
            self.idle = 0;
        }
    }

    fn on_event(&mut self, _window: &mut baseview::Window, event: Event) -> EventStatus {
        match event {
            Event::Mouse(event) => {
                let event = match event {
                    MouseEvent::CursorMoved {
                        position,
                        modifiers,
                    } => {
                        self.sync_modifiers(modifiers);
                        self.cursor = LogicalPosition::new(position.x as f32, position.y as f32);
                        WindowEvent::PointerMoved {
                            position: self.cursor,
                        }
                    }
                    MouseEvent::ButtonPressed { button, modifiers } => {
                        self.sync_modifiers(modifiers);
                        WindowEvent::PointerPressed {
                            position: self.cursor,
                            button: map_button(button),
                        }
                    }
                    MouseEvent::ButtonReleased { button, modifiers } => {
                        self.sync_modifiers(modifiers);
                        WindowEvent::PointerReleased {
                            position: self.cursor,
                            button: map_button(button),
                        }
                    }
                    MouseEvent::WheelScrolled { delta, modifiers } => {
                        self.sync_modifiers(modifiers);
                        let (delta_x, delta_y) = match delta {
                            ScrollDelta::Lines { x, y } => (x * 40.0, y * 40.0),
                            ScrollDelta::Pixels { x, y } => (x, y),
                        };
                        WindowEvent::PointerScrolled {
                            position: self.cursor,
                            delta_x,
                            delta_y,
                        }
                    }
                    MouseEvent::CursorLeft => WindowEvent::PointerExited,
                    _ => return EventStatus::Ignored,
                };
                self.window.dispatch_event(event);
                EventStatus::Captured
            }
            Event::Window(baseview::WindowEvent::Resized(info)) => {
                let size = info.physical_size();
                self.scale = info.scale() as f32;
                self.size = (size.width.max(1), size.height.max(1));
                let _ = self.surface.resize(
                    NonZeroU32::new(self.size.0).unwrap(),
                    NonZeroU32::new(self.size.1).unwrap(),
                );
                self.window.dispatch_event(WindowEvent::ScaleFactorChanged {
                    scale_factor: self.scale,
                });
                self.window
                    .set_size(PhysicalSize::new(self.size.0, self.size.1));
                self.dirty = true;
                self.window.request_redraw();
                EventStatus::Captured
            }
            // let the host keep its shortcuts
            _ => EventStatus::Ignored,
        }
    }
}

impl Drop for EditorWindow {
    fn drop(&mut self) {
        let _ = self.ui.hide();
        let _ = self.window.window().hide();
    }
}
