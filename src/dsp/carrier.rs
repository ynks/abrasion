use std::f32::consts::{PI, TAU};

use crate::params::Shape;

// uniform noise in -1..1
pub struct Rng(u32);

impl Rng {
    pub fn new(seed: u32) -> Self {
        Self(seed.max(1))
    }

    pub fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0 as f32 / u32::MAX as f32 * 2.0 - 1.0
    }
}

// uniform noise rms
const NOISE_RMS: f32 = 0.577_350_3;
// carrier rms before the soft clip
const TARGET_RMS: f32 = 0.5;
const MAX_GAIN: f32 = 400.0;

fn poly_blep(t: f32, dt: f32) -> f32 {
    if t < dt {
        let t = t / dt;
        t + t - t * t - 1.0
    } else if t > 1.0 - dt {
        let t = (t - 1.0) / dt;
        t * t + t + t + 1.0
    } else {
        0.0
    }
}

// one sample of the periodic carrier, phase in 0..1
pub fn wave(shape: Shape, phase: f32, dt: f32) -> f32 {
    match shape {
        Shape::Sine => (TAU * phase).sin(),
        Shape::Triangle => {
            let t = phase + 0.25;
            let t = t - t.floor();
            4.0 * (t - 0.5).abs() - 1.0
        }
        Shape::Square => {
            let dt = dt.clamp(1e-6, 0.5);
            let naive = if phase < 0.5 { 1.0 } else { -1.0 };
            let half = phase + 0.5;
            naive + poly_blep(phase, dt) - poly_blep(half - half.floor(), dt)
        }
    }
}

// bandwidth in octaves to q
pub fn octaves_to_q(bw: f32) -> f32 {
    let r = 2f32.powf(bw.max(0.01));
    r.sqrt() / (r - 1.0)
}

// tpt state variable bandpass with unity peak gain
#[derive(Clone, Copy, Default)]
pub struct Svf {
    ic1: f32,
    ic2: f32,
}

#[derive(Clone, Copy, Default)]
pub struct SvfCoefs {
    k: f32,
    a1: f32,
    a2: f32,
    a3: f32,
    // keeps the output near TARGET_RMS
    pub gain: f32,
}

impl SvfCoefs {
    pub fn bandpass(freq: f32, bw: f32, sr: f32) -> Self {
        let freq = freq.clamp(10.0, sr * 0.45);
        let q = octaves_to_q(bw);
        let g = (PI * freq / sr).tan();
        let k = 1.0 / q;
        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        let a3 = g * a2;

        // noise power passed by a unity peak bandpass is about pi f / (q sr)
        let pass = (PI * freq / (q * sr)).min(1.0);
        let gain = (TARGET_RMS / (NOISE_RMS * pass.sqrt())).min(MAX_GAIN);
        Self {
            k,
            a1,
            a2,
            a3,
            gain,
        }
    }
}

impl Svf {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn process(&mut self, x: f32, c: &SvfCoefs) -> f32 {
        let v3 = x - self.ic2;
        let v1 = c.a1 * self.ic1 + c.a2 * v3;
        let v2 = self.ic2 + c.a2 * self.ic1 + c.a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        c.k * v1
    }
}

// fast random wobble, output in -1..1
pub struct Drift {
    rng: Rng,
    target: f32,
    value: f32,
    countdown: f32,
}

const DRIFT_MIN_RATE: f32 = 8.0;
const DRIFT_MAX_RATE: f32 = 40.0;
const DRIFT_GLIDE: f32 = 0.010;

impl Drift {
    pub fn new(seed: u32) -> Self {
        Self {
            rng: Rng::new(seed),
            target: 0.0,
            value: 0.0,
            countdown: 0.0,
        }
    }

    pub fn reset(&mut self) {
        self.target = 0.0;
        self.value = 0.0;
        self.countdown = 0.0;
    }

    pub fn next(&mut self, amount: f32, sr: f32) -> f32 {
        if amount <= 0.0 {
            self.value = 0.0;
            self.target = 0.0;
            return 0.0;
        }
        self.countdown -= 1.0;
        if self.countdown <= 0.0 {
            let rate = DRIFT_MIN_RATE + amount * (DRIFT_MAX_RATE - DRIFT_MIN_RATE);
            self.countdown += sr / rate;
            self.target = self.rng.next();
        }
        // one pole, close enough to 1 - exp(-x) at this size
        let k = (1.0 / (DRIFT_GLIDE * sr)).min(1.0);
        self.value += (self.target - self.value) * k;
        self.value
    }
}
