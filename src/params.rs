use nih_plug::prelude::*;
use std::sync::Arc;

use crate::editor::EditorState;

pub const MIN_FREQ: f32 = 30.0;
pub const MAX_FREQ: f32 = 18_000.0;
pub const AXIS_MIN: f32 = 20.0;
pub const AXIS_MAX: f32 = 20_000.0;

#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    #[name = "Sine"]
    Sine,
    #[name = "Triangle"]
    Triangle,
    #[name = "Square"]
    Square,
}

#[derive(Params)]
pub struct AbrParams {
    #[persist = "editor-state"]
    pub editor_state: Arc<EditorState>,

    #[id = "freq"]
    pub freq: FloatParam,
    #[id = "amount"]
    pub amount: FloatParam,
    #[id = "blend"]
    pub blend: FloatParam,
    #[id = "width"]
    pub width: FloatParam,
    #[id = "stereo"]
    pub stereo: FloatParam,
    #[id = "drift"]
    pub drift: FloatParam,
    #[id = "shape"]
    pub shape: EnumParam<Shape>,
}

fn percent_param(name: &str, default: f32, decimals: usize) -> FloatParam {
    FloatParam::new(name, default, FloatRange::Linear { min: 0.0, max: 1.0 })
        .with_smoother(SmoothingStyle::Linear(20.0))
        .with_unit("%")
        .with_value_to_string(formatters::v2s_f32_percentage(decimals))
        .with_string_to_value(formatters::s2v_f32_percentage())
}

impl Default for AbrParams {
    fn default() -> Self {
        Self {
            editor_state: EditorState::from_size(crate::editor::DEFAULT_SIZE),

            freq: FloatParam::new(
                "Freq",
                2000.0,
                FloatRange::Skewed {
                    min: MIN_FREQ,
                    max: MAX_FREQ,
                    factor: FloatRange::skew_factor(-2.2),
                },
            )
            .with_smoother(SmoothingStyle::Logarithmic(20.0))
            .with_value_to_string(formatters::v2s_f32_hz_then_khz(2))
            .with_string_to_value(formatters::s2v_f32_hz_then_khz()),
            amount: percent_param("Amount", 0.3, 0),
            blend: percent_param("Noise Blend", 0.0, 0),
            width: FloatParam::new(
                "Filter Width",
                1.0,
                FloatRange::Skewed {
                    min: 0.1,
                    max: 10.0,
                    factor: FloatRange::skew_factor(-1.5),
                },
            )
            .with_smoother(SmoothingStyle::Linear(20.0))
            .with_unit(" oct")
            .with_value_to_string(formatters::v2s_f32_rounded(2)),
            stereo: percent_param("Stereo Width", 0.0, 1),
            drift: percent_param("Drift", 0.0, 0),
            shape: EnumParam::new("Shape", Shape::Sine),
        }
    }
}

impl AbrParams {
    // same order as parameters in the slint theme
    pub fn ui_order(&self) -> Vec<ParamPtr> {
        vec![
            self.freq.as_ptr(),
            self.amount.as_ptr(),
            self.blend.as_ptr(),
            self.width.as_ptr(),
            self.stereo.as_ptr(),
            self.drift.as_ptr(),
            self.shape.as_ptr(),
        ]
    }
}

// 20 Hz to 20 kHz
pub fn freq_to_pos(freq: f32) -> f32 {
    (freq.max(1.0) / AXIS_MIN).ln() / (AXIS_MAX / AXIS_MIN).ln()
}

pub fn pos_to_freq(pos: f32) -> f32 {
    AXIS_MIN * (AXIS_MAX / AXIS_MIN).powf(pos)
}
