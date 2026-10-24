use slint::{Rgb8Pixel, SharedPixelBuffer};
use std::f32::consts::{LN_2, PI};

use super::analyzer::POINTS;
use crate::dsp::carrier::wave;
use crate::params::{freq_to_pos, Shape, AXIS_MAX, AXIS_MIN};

pub const VIEW_WIDTH: f32 = 508.0;
pub const VIEW_HEIGHT: f32 = 200.0;

const ROWS: usize = 24;
const ROW_INTERVAL: f32 = 1.0 / 30.0;
const DB_MIN: f32 = -90.0;
const DB_MAX: f32 = 0.0;

type Rgb = [f32; 3];
const BG: Rgb = [20.0, 22.0, 25.0];
const ACCENT: Rgb = [255.0, 77.0, 61.0];
const GREY: Rgb = [199.0, 201.0, 203.0];

#[derive(Clone, Copy, PartialEq)]
pub struct ViewParams {
    pub f_eff: f32,
    pub amount: f32,
    pub blend: f32,
    pub width: f32,
    pub stereo: f32,
    pub shape: Shape,
}

fn hash(i: u32, k: u32) -> f32 {
    let mut h = i
        .wrapping_mul(374_761_393)
        .wrapping_add(k.wrapping_mul(668_265_263))
        .wrapping_mul(1_274_126_177);
    h ^= h >> 13;
    h = h.wrapping_mul(1_274_126_177);
    (h ^ (h >> 16)) as f32 / u32::MAX as f32
}

fn px(c: Rgb) -> Rgb8Pixel {
    Rgb8Pixel::new(c[0] as u8, c[1] as u8, c[2] as u8)
}

fn blend(p: &mut Rgb8Pixel, c: Rgb, a: f32) {
    if a <= 0.0 {
        return;
    }
    let a = a.min(1.0);
    p.r = (p.r as f32 + (c[0] - p.r as f32) * a) as u8;
    p.g = (p.g as f32 + (c[1] - p.g as f32) * a) as u8;
    p.b = (p.b as f32 + (c[2] - p.b as f32) * a) as u8;
}

pub struct StrataRenderer {
    // newest row at head
    rows: [[f32; POINTS]; ROWS],
    head: usize,
    filled: usize,
    since_row: f32,
    time: f32,
    frame: u32,

    ys: [f32; POINTS],
    broken: [bool; POINTS],
    col_y: Vec<f32>,
    col_broken: Vec<bool>,
}

impl StrataRenderer {
    pub fn new() -> Self {
        Self {
            rows: [[0.0; POINTS]; ROWS],
            head: 0,
            filled: 0,
            since_row: ROW_INTERVAL,
            time: 0.0,
            frame: 0,
            ys: [0.0; POINTS],
            broken: [false; POINTS],
            col_y: Vec::new(),
            col_broken: Vec::new(),
        }
    }

    // advances time and takes a new history row when one is due
    pub fn tick(&mut self, spectrum_db: &[f32; POINTS], dt: f32) {
        self.time += dt;
        self.frame = self.frame.wrapping_add(1);
        self.since_row += dt;
        if self.since_row < ROW_INTERVAL {
            return;
        }
        self.since_row = (self.since_row - ROW_INTERVAL).min(ROW_INTERVAL);
        self.head = (self.head + 1) % ROWS;
        self.filled = (self.filled + 1).min(ROWS);
        for (h, db) in self.rows[self.head].iter_mut().zip(spectrum_db) {
            *h = ((db - DB_MIN) / (DB_MAX - DB_MIN)).clamp(0.03, 0.95);
        }
    }

    pub fn render(
        &mut self,
        width: usize,
        height: usize,
        view: &ViewParams,
    ) -> SharedPixelBuffer<Rgb8Pixel> {
        let (w, h) = (width.max(2), height.max(2));
        let scale = w as f32 / VIEW_WIDTH;
        let (vw, vh) = (VIEW_WIDTH, VIEW_HEIGHT);

        let mut img = SharedPixelBuffer::<Rgb8Pixel>::new(w as u32, h as u32);
        let pixels = img.make_mut_slice();
        pixels.fill(px(BG));

        // filter band in axis units, the axis spans log2(1000) octaves
        let axis_oct = (AXIS_MAX / AXIS_MIN).ln() / LN_2;
        let fpos = freq_to_pos(view.f_eff);
        let bw = view.width * 0.5 / axis_oct;
        let (lo, hi) = (fpos - bw, fpos + bw);
        let sine_amt = (1.0 - view.blend) * view.amount;
        let noise_amt = view.blend * view.amount;
        let env = |x: f32| (-((x - fpos) / (bw * 1.1 + 1e-3)).powi(2) * 0.5).exp();
        let senv = |x: f32| (-((x - fpos) / 0.12).powi(2) * 0.5).exp();
        let kx = 10.0 + fpos * 70.0;
        let t = self.time * 0.9;

        let rows = self.filled;
        for k in (0..rows).rev() {
            let row = self.rows[(self.head + ROWS - k) % ROWS];
            let front = k == 0;
            let off = k as f32 * 5.6;
            let sc = 1.0 - k as f32 * 0.022;
            let alpha = if front {
                1.0
            } else {
                (0.72 - k as f32 * 0.028).max(0.08)
            };
            let seed = self.frame.wrapping_add(k as u32 * 17);

            for i in 0..POINTS {
                let x = i as f32 / (POINTS - 1) as f32;
                let mut y = vh - 8.0 - row[i] * vh * 0.66 * sc - off;
                let phase = (x * kx - t + k as f32 * 0.09).rem_euclid(1.0);
                y += sine_amt * senv(x) * 13.0 * wave(view.shape, phase, 0.0);

                self.broken[i] = false;
                if noise_amt > 0.0 && x > lo && x < hi {
                    let e = env(x) * noise_amt;
                    y += (hash(i as u32, seed) - 0.5) * e * 34.0;
                    let kseed = self.frame.wrapping_sub(k as u32);
                    self.broken[i] = hash(i as u32 + 999, kseed) < e * 0.8;
                }
                self.ys[i] = y;
            }

            let split = front && view.stereo > 0.01;
            let passes: &[(f32, Rgb, f32)] = if split {
                &[(-1.0, ACCENT, 1.0), (1.0, GREY, 0.55)]
            } else if front {
                &[(0.0, ACCENT, 1.0)]
            } else {
                &[(0.0, GREY, 1.0)]
            };
            for &(side, color, a) in passes {
                self.columns(w, scale, |x, y| {
                    if side == 0.0 {
                        y
                    } else {
                        let swing = (x * kx * PI + t * 3.0).sin();
                        y + side * view.stereo * view.amount * senv(x) * 6.0 * swing
                    }
                });
                let half = if front { 0.8 } else { 0.5 } * scale;
                line(
                    pixels,
                    w,
                    h,
                    &self.col_y,
                    &self.col_broken,
                    color,
                    alpha * a,
                    half,
                );
            }

            // debris from the broken bits
            if noise_amt > 0.01 {
                let color = if front { ACCENT } else { GREY };
                let size = (1.5 * scale).max(1.0);
                for i in 0..POINTS {
                    if !self.broken[i] {
                        continue;
                    }
                    let x = i as f32 / (POINTS - 1) as f32 * vw * scale;
                    let fall = hash(i as u32, self.frame.wrapping_mul(3).wrapping_add(k as u32));
                    let y = (self.ys[i] + fall * 14.0 * noise_amt) * scale;
                    rect(pixels, w, h, (x, y), (x + size, y + size), color, alpha * 0.9);
                }
            }
        }

        // sine line, solid and vertical
        let fx = fpos * vw * scale;
        let sw = 0.75 * scale;
        rect(
            pixels,
            w,
            h,
            (fx - sw, 0.0),
            (fx + sw, h as f32),
            ACCENT,
            0.2 + 0.8 * (1.0 - view.blend),
        );

        // noise lines, dotted and horizontal over the band
        if view.blend > 0.005 {
            let yc = vh * (1.0 - view.amount) * scale;
            let a = 0.15 + 0.75 * view.blend;
            let (x0, x1) = (lo * vw * scale, hi * vw * scale);
            let dash = 2.0 * scale;
            for dy in [-8.0, 0.0, 8.0] {
                let y = yc + dy * scale;
                let mut x = x0;
                while x < x1 {
                    rect(pixels, w, h, (x, y), ((x + dash).min(x1), y + scale), ACCENT, a);
                    x += dash * 3.0;
                }
            }
            let tick = 12.0 * scale;
            let ta = 0.3 + 0.6 * view.blend;
            rect(pixels, w, h, (x0, yc - tick), (x0 + scale, yc + tick), ACCENT, ta);
            rect(pixels, w, h, (x1, yc - tick), (x1 + scale, yc + tick), ACCENT, ta);
        }

        img
    }

    // resamples the points to one y per pixel column
    fn columns(&mut self, w: usize, scale: f32, shift: impl Fn(f32, f32) -> f32) {
        self.col_y.clear();
        self.col_broken.clear();
        let last = (POINTS - 1) as f32;
        for c in 0..w {
            let p = (c as f32 + 0.5) / w as f32 * last;
            let i = (p.floor() as usize).min(POINTS - 2);
            let t = p - i as f32;
            let x = p / last;
            let y = self.ys[i] + (self.ys[i + 1] - self.ys[i]) * t;
            self.col_y.push(shift(x, y) * scale);
            self.col_broken
                .push(self.broken[if t < 0.5 { i } else { i + 1 }]);
        }
    }
}

fn rect(
    pixels: &mut [Rgb8Pixel],
    w: usize,
    h: usize,
    from: (f32, f32),
    to: (f32, f32),
    color: Rgb,
    alpha: f32,
) {
    let x0 = from.0.round().max(0.0) as usize;
    let y0 = from.1.round().max(0.0) as usize;
    let x1 = (to.0.round().max(from.0.round() + 1.0) as usize).min(w);
    let y1 = (to.1.round().max(from.1.round() + 1.0) as usize).min(h);
    for y in y0..y1 {
        for p in &mut pixels[y * w + x0.min(x1)..y * w + x1] {
            blend(p, color, alpha);
        }
    }
}

// antialiased line with one y per pixel column, skipping broken columns
#[allow(clippy::too_many_arguments)]
fn line(
    pixels: &mut [Rgb8Pixel],
    w: usize,
    h: usize,
    ys: &[f32],
    broken: &[bool],
    color: Rgb,
    alpha: f32,
    half: f32,
) {
    let reach = half + 1.0;
    for x in 0..w {
        let prev = x.saturating_sub(1);
        if broken[x] || broken[prev] {
            continue;
        }
        let (a, b) = (ys[prev], ys[x]);
        let (top, bottom) = if a < b { (a, b) } else { (b, a) };
        let start = (top - reach).floor().max(0.0) as usize;
        let end = ((bottom + reach).ceil().max(0.0) as usize).min(h);
        for y in start..end {
            let yc = y as f32 + 0.5;
            let d = (top - yc).max(yc - bottom).max(0.0);
            let cover = (half + 0.5 - d).clamp(0.0, 1.0);
            blend(&mut pixels[y * w + x], color, cover * alpha);
        }
    }
}
