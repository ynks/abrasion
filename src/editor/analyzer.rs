use realfft::num_complex::Complex;
use realfft::{RealFftPlanner, RealToComplex};
use std::f32::consts::TAU;
use std::sync::Arc;

use crate::params::{pos_to_freq, AXIS_MAX, AXIS_MIN};
use crate::shared::{ScopeFrame, SCOPE_SIZE};

pub const POINTS: usize = 180;
const FLOOR_DB: f32 = -120.0;

// input spectrum as log spaced points in dB
pub struct Analyzer {
    fft: Arc<dyn RealToComplex<f32>>,
    window: Vec<f32>,
    input: Vec<f32>,
    spectrum: Vec<Complex<f32>>,
    scratch: Vec<Complex<f32>>,
    bin_db: Vec<f32>,
    // first last and center bin per point
    ranges: Vec<(usize, usize, f32)>,
    range_sr: f32,
    points: [f32; POINTS],
}

impl Analyzer {
    pub fn new() -> Self {
        let fft = RealFftPlanner::<f32>::new().plan_fft_forward(SCOPE_SIZE);
        let window: Vec<f32> = (0..SCOPE_SIZE)
            .map(|i| 0.5 - 0.5 * (TAU * i as f32 / SCOPE_SIZE as f32).cos())
            .collect();
        Self {
            input: fft.make_input_vec(),
            spectrum: fft.make_output_vec(),
            scratch: fft.make_scratch_vec(),
            bin_db: vec![FLOOR_DB; SCOPE_SIZE / 2 + 1],
            fft,
            window,
            ranges: Vec::with_capacity(POINTS),
            range_sr: 0.0,
            points: [FLOOR_DB; POINTS],
        }
    }

    pub fn points(&self) -> &[f32; POINTS] {
        &self.points
    }

    fn update_ranges(&mut self, sr: f32) {
        if self.range_sr == sr {
            return;
        }
        self.range_sr = sr;
        self.ranges.clear();

        let bins = self.bin_db.len();
        let bin_hz = sr / SCOPE_SIZE as f32;
        let half = 0.5 / POINTS as f32;
        for p in 0..POINTS {
            let pos = (p as f32 + 0.5) / POINTS as f32;
            let lo = pos_to_freq((pos - half).max(0.0)).max(AXIS_MIN) / bin_hz;
            let hi = pos_to_freq((pos + half).min(1.0)).min(AXIS_MAX) / bin_hz;
            let center = (pos_to_freq(pos) / bin_hz).min((bins - 1) as f32);
            let first = lo.ceil() as usize;
            let last = (hi.floor() as usize).min(bins - 1);
            self.ranges.push((first, last, center));
        }
    }

    pub fn feed(&mut self, frame: &ScopeFrame) {
        self.update_ranges(frame.sample_rate);

        for ((x, s), w) in self.input.iter_mut().zip(&frame.samples).zip(&self.window) {
            *x = s * w;
        }
        if self
            .fft
            .process_with_scratch(&mut self.input, &mut self.spectrum, &mut self.scratch)
            .is_err()
        {
            return;
        }

        // full scale sine reads 0 dB
        let scale = 4.0 / SCOPE_SIZE as f32;
        for (db, c) in self.bin_db.iter_mut().zip(&self.spectrum) {
            let mag = c.norm() * scale;
            *db = (20.0 * mag.max(1e-9).log10()).max(FLOOR_DB);
        }

        for (p, &(first, last, center)) in self.ranges.iter().enumerate() {
            let db = if first <= last {
                self.bin_db[first..=last]
                    .iter()
                    .copied()
                    .fold(FLOOR_DB, f32::max)
            } else {
                // low end has fewer bins than points so interpolate
                let k = center.floor() as usize;
                let k1 = (k + 1).min(self.bin_db.len() - 1);
                let t = center - k as f32;
                self.bin_db[k] + (self.bin_db[k1] - self.bin_db[k]) * t
            };
            let s = &mut self.points[p];
            let k = if db > *s { 0.6 } else { 0.15 };
            *s += (db - *s) * k;
        }
    }
}
