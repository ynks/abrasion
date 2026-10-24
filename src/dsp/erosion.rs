use super::carrier::{wave, Drift, Rng, Svf, SvfCoefs};
use crate::params::Shape;

pub const BASE_DELAY_S: f32 = 0.002; // reported latency
const MAX_DEPTH: f32 = 0.5; // delay swing at full amount
const DRIFT_OCTAVES: f32 = 1.0;
const STEREO_PHASE: f32 = 0.25;
const COEF_INTERVAL: u32 = 16;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    pub freq: f32,
    pub amount: f32,
    pub blend: f32,
    pub width: f32,
    pub stereo: f32,
    pub drift: f32,
    pub shape: Shape,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            freq: 2000.0,
            amount: 0.3,
            blend: 0.0,
            width: 1.0,
            stereo: 0.0,
            drift: 0.0,
            shape: Shape::Sine,
        }
    }
}

pub struct Erosion {
    sr: f32,
    channels: usize,
    base: usize,

    lines: Vec<Vec<f32>>,
    mask: usize,
    write: usize,

    phase: f32,
    noise: [Rng; 2],
    svf: [Svf; 2],
    coefs: SvfCoefs,
    coef_key: (f32, f32),
    coef_countdown: u32,
    drift: Drift,
    f_eff: f32,
}

impl Erosion {
    pub fn new(channels: usize, sr: f32) -> Self {
        let channels = channels.clamp(1, 2);
        let base = (BASE_DELAY_S * sr).round().max(2.0) as usize;
        let len = (base * 2 + 4).next_power_of_two();
        Self {
            sr,
            channels,
            base,
            lines: vec![vec![0.0; len]; channels],
            mask: len - 1,
            write: 0,
            phase: 0.0,
            noise: [Rng::new(0x9e37_79b9), Rng::new(0x85eb_ca6b)],
            svf: [Svf::default(); 2],
            coefs: SvfCoefs::default(),
            coef_key: (0.0, 0.0),
            coef_countdown: 0,
            drift: Drift::new(0xc2b2_ae35),
            f_eff: 0.0,
        }
    }

    pub fn latency(&self) -> usize {
        self.base
    }

    // carrier frequency after drift
    pub fn f_eff(&self) -> f32 {
        self.f_eff
    }

    pub fn reset(&mut self) {
        for line in &mut self.lines {
            line.fill(0.0);
        }
        self.write = 0;
        self.phase = 0.0;
        for s in &mut self.svf {
            s.reset();
        }
        self.drift.reset();
        self.coef_countdown = 0;
    }

    fn update_coefs(&mut self, freq: f32, width: f32) {
        if self.coef_countdown > 0 && self.coef_key == (freq, width) {
            self.coef_countdown -= 1;
            return;
        }
        self.coef_countdown = COEF_INTERVAL - 1;
        if self.coef_key != (freq, width) {
            self.coef_key = (freq, width);
            self.coefs = SvfCoefs::bandpass(freq, width, self.sr);
        }
    }

    // processes one frame in place
    pub fn process(&mut self, frame: &mut [f32], s: &Settings) {
        let sr = self.sr;
        let walk = self.drift.next(s.drift, sr);
        let freq = (s.freq * 2f32.powf(s.drift * DRIFT_OCTAVES * walk)).clamp(20.0, sr * 0.45);
        self.f_eff = freq;

        let dt = freq / sr;
        self.phase += dt;
        self.phase -= self.phase.floor();

        let noisy = s.blend > 0.0;
        if noisy {
            self.update_coefs(freq, s.width);
        }
        let (w0, w1) = (self.noise[0].next(), self.noise[1].next());

        let depth = s.amount * s.amount * MAX_DEPTH;
        let base = self.base as f32;

        for (c, x) in frame.iter_mut().take(self.channels).enumerate() {
            let line = &mut self.lines[c];
            line[self.write] = *x;

            let right = c == 1;
            let mut phase = self.phase;
            if right {
                phase += s.stereo * STEREO_PHASE;
                phase -= phase.floor();
            }
            let osc = wave(s.shape, phase, dt);

            // right channel morphs from shared to independent noise
            let white = if right {
                let mixed = w0 * (1.0 - s.stereo) + w1 * s.stereo;
                mixed / ((1.0 - s.stereo).powi(2) + s.stereo * s.stereo).sqrt()
            } else {
                w0
            };
            let noise = if noisy {
                (self.svf[c].process(white, &self.coefs) * self.coefs.gain).tanh()
            } else {
                0.0
            };

            let m = osc * (1.0 - s.blend) + noise * s.blend;
            let d = (base * (1.0 + depth * m)).clamp(1.0, base * 2.0);

            let pos = self.write as f32 - d + line.len() as f32;
            let i = pos.floor();
            let t = pos - i;
            let i = i as usize;
            let a = line[i & self.mask];
            let b = line[(i + 1) & self.mask];
            *x = a + (b - a) * t;
        }

        self.write = (self.write + 1) & self.mask;
    }
}
