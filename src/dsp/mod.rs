pub mod carrier;
pub mod erosion;

#[cfg(test)]
mod tests {
    use super::carrier::{Rng, Svf, SvfCoefs};
    use super::erosion::{Erosion, Settings};
    use crate::params::Shape;
    use std::f32::consts::TAU;

    const SR: f32 = 48000.0;

    fn input(i: usize) -> f32 {
        0.5 * (TAU * 220.0 * i as f32 / SR).sin() + 0.3 * (TAU * 3100.0 * i as f32 / SR).sin()
    }

    fn run(s: Settings, len: usize) -> Vec<[f32; 2]> {
        let mut e = Erosion::new(2, SR);
        (0..len)
            .map(|i| {
                let mut frame = [input(i), input(i)];
                e.process(&mut frame, &s);
                frame
            })
            .collect()
    }

    fn max_diff(a: &[[f32; 2]], b: &[[f32; 2]], ch: usize) -> f32 {
        a.iter()
            .zip(b)
            .map(|(x, y)| (x[ch] - y[ch]).abs())
            .fold(0.0, f32::max)
    }

    #[test]
    fn zero_amount_is_a_clean_delay() {
        let s = Settings {
            amount: 0.0,
            blend: 0.6,
            stereo: 1.0,
            drift: 1.0,
            ..Settings::default()
        };
        let mut e = Erosion::new(2, SR);
        let lat = e.latency();
        assert_eq!(lat, 96);
        for i in 0..4000 {
            let mut frame = [input(i), -input(i)];
            e.process(&mut frame, &s);
            let want = if i >= lat { input(i - lat) } else { 0.0 };
            assert!((frame[0] - want).abs() < 1e-6, "sample {i}");
            assert!((frame[1] + want).abs() < 1e-6, "sample {i}");
        }
    }

    #[test]
    fn width_does_nothing_without_noise() {
        let narrow = run(
            Settings {
                width: 0.1,
                amount: 0.8,
                ..Settings::default()
            },
            4000,
        );
        let wide = run(
            Settings {
                width: 10.0,
                amount: 0.8,
                ..Settings::default()
            },
            4000,
        );
        assert_eq!(max_diff(&narrow, &wide, 0), 0.0);
    }

    #[test]
    fn modulation_changes_the_signal() {
        let dry = run(
            Settings {
                amount: 0.0,
                ..Settings::default()
            },
            4000,
        );
        let wet = run(
            Settings {
                amount: 0.8,
                ..Settings::default()
            },
            4000,
        );
        assert!(max_diff(&dry, &wet, 0) > 0.05);
    }

    #[test]
    fn stereo_width_splits_channels() {
        for blend in [0.0, 0.5, 1.0] {
            let mono = run(
                Settings {
                    amount: 0.8,
                    blend,
                    stereo: 0.0,
                    ..Settings::default()
                },
                4000,
            );
            assert!(
                mono.iter().all(|f| f[0] == f[1]),
                "blend {blend} should stay mono"
            );

            let wide = run(
                Settings {
                    amount: 0.8,
                    blend,
                    stereo: 1.0,
                    ..Settings::default()
                },
                4000,
            );
            let diff = wide.iter().map(|f| (f[0] - f[1]).abs()).fold(0.0, f32::max);
            assert!(diff > 0.01, "blend {blend} should be wide, diff {diff}");
        }
    }

    #[test]
    fn noise_carrier_level_is_steady() {
        let mut levels = Vec::new();
        for freq in [100.0, 1000.0, 10_000.0] {
            for bw in [0.1, 1.0, 10.0] {
                let c = SvfCoefs::bandpass(freq, bw, SR);
                let mut svf = Svf::default();
                let mut rng = Rng::new(7);
                let n = 96_000;
                let mut sum = 0.0;
                for i in 0..n {
                    let y = (svf.process(rng.next(), &c) * c.gain).tanh();
                    if i >= n / 4 {
                        sum += y * y;
                    }
                }
                let rms = (sum / (n - n / 4) as f32).sqrt();
                levels.push((freq, bw, rms));
            }
        }
        let lo = levels.iter().map(|l| l.2).fold(f32::INFINITY, f32::min);
        let hi = levels.iter().map(|l| l.2).fold(0.0, f32::max);
        assert!(hi / lo < 2.0, "noise levels spread too far: {levels:?}");
    }

    #[test]
    fn extreme_settings_stay_bounded() {
        for shape in [Shape::Sine, Shape::Triangle, Shape::Square] {
            let out = run(
                Settings {
                    freq: 18_000.0,
                    amount: 1.0,
                    blend: 0.5,
                    width: 10.0,
                    stereo: 1.0,
                    drift: 1.0,
                    shape,
                },
                8000,
            );
            for f in &out {
                assert!(f[0].is_finite() && f[1].is_finite());
                assert!(f[0].abs() <= 0.81 && f[1].abs() <= 0.81);
            }
        }
    }

    #[test]
    fn shapes_sound_different() {
        let base = Settings {
            freq: 500.0,
            amount: 0.8,
            ..Settings::default()
        };
        let sine = run(base, 4000);
        let tri = run(
            Settings {
                shape: Shape::Triangle,
                ..base
            },
            4000,
        );
        let square = run(
            Settings {
                shape: Shape::Square,
                ..base
            },
            4000,
        );
        assert!(max_diff(&sine, &tri, 0) > 1e-3);
        assert!(max_diff(&sine, &square, 0) > 1e-3);
        assert!(max_diff(&tri, &square, 0) > 1e-3);
    }

    #[test]
    fn drift_moves_the_carrier() {
        let mut e = Erosion::new(1, SR);
        let still = Settings {
            freq: 1000.0,
            ..Settings::default()
        };
        for _ in 0..1000 {
            e.process(&mut [0.0], &still);
            assert_eq!(e.f_eff(), 1000.0);
        }

        let moving = Settings {
            drift: 1.0,
            ..still
        };
        let (mut lo, mut hi) = (f32::INFINITY, 0.0f32);
        for _ in 0..48_000 {
            e.process(&mut [0.0], &moving);
            lo = lo.min(e.f_eff());
            hi = hi.max(e.f_eff());
        }
        // fast wobble should cover a good part of the octave either way
        assert!(lo < 800.0 && hi > 1250.0, "range {lo}..{hi}");
        assert!(lo >= 499.0 && hi <= 2001.0, "range {lo}..{hi}");
    }
}
