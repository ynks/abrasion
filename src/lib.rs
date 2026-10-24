use nih_plug::prelude::*;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

mod dsp;
mod editor;
mod params;
mod shared;

use dsp::erosion::{Erosion, Settings};
use params::AbrParams;
use shared::{Live, Scope, ScopeFrame};

pub struct Abrasion {
    params: Arc<AbrParams>,
    sample_rate: f32,
    channels: usize,

    erosion: Erosion,

    scope: Scope,
    frame_in: triple_buffer::Input<ScopeFrame>,
    frame_out: Arc<Mutex<triple_buffer::Output<ScopeFrame>>>,
    live: Arc<Live>,

    // test signal for the standalone build
    #[cfg(feature = "standalone")]
    demo: Option<demo::Demo>,
}

#[cfg(feature = "standalone")]
mod demo {
    use std::f32::consts::TAU;

    pub struct Demo {
        phase: [f32; 4],
        lfo: f32,
        seed: u32,
    }

    impl Demo {
        pub fn new() -> Self {
            Self {
                phase: [0.0; 4],
                lfo: 0.0,
                seed: 0x1234_5678,
            }
        }

        pub fn next(&mut self, sr: f32) -> f32 {
            const FREQS: [f32; 4] = [110.0, 440.0, 1250.0, 3600.0];
            const AMPS: [f32; 4] = [0.25, 0.12, 0.05, 0.02];

            self.lfo = (self.lfo + 0.25 / sr).fract();
            let mut out = 0.0;
            for (i, p) in self.phase.iter_mut().enumerate() {
                *p = (*p + FREQS[i] / sr).fract();
                let trem = 0.5 + 0.5 * (TAU * (self.lfo + i as f32 * 0.25)).sin();
                out += (TAU * *p).sin() * AMPS[i] * trem;
            }

            self.seed ^= self.seed << 13;
            self.seed ^= self.seed >> 17;
            self.seed ^= self.seed << 5;
            out + (self.seed as f32 / u32::MAX as f32 - 0.5) * 0.004
        }
    }
}

impl Default for Abrasion {
    fn default() -> Self {
        let (frame_in, frame_out) = triple_buffer::triple_buffer(&ScopeFrame::default());
        Self {
            params: Arc::new(AbrParams::default()),
            sample_rate: 48000.0,
            channels: 2,
            erosion: Erosion::new(2, 48000.0),
            scope: Scope::new(),
            frame_in,
            frame_out: Arc::new(Mutex::new(frame_out)),
            live: Arc::new(Live::new()),
            #[cfg(feature = "standalone")]
            demo: std::env::var_os("ABR_DEMO").map(|_| demo::Demo::new()),
        }
    }
}

impl Plugin for Abrasion {
    const NAME: &'static str = "Abrasion";
    const VENDOR: &'static str = "xein";
    const URL: &'static str = "www.xein.es";
    const EMAIL: &'static str = "xgonip@gmail.com";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[
        AudioIOLayout {
            main_input_channels: NonZeroU32::new(2),
            main_output_channels: NonZeroU32::new(2),
            ..AudioIOLayout::const_default()
        },
        AudioIOLayout {
            main_input_channels: NonZeroU32::new(1),
            main_output_channels: NonZeroU32::new(1),
            ..AudioIOLayout::const_default()
        },
    ];

    const SAMPLE_ACCURATE_AUTOMATION: bool = false;

    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        editor::create(
            self.params.clone(),
            self.frame_out.clone(),
            self.live.clone(),
        )
    }

    fn initialize(
        &mut self,
        layout: &AudioIOLayout,
        config: &BufferConfig,
        context: &mut impl InitContext<Self>,
    ) -> bool {
        self.sample_rate = config.sample_rate;
        self.channels = layout
            .main_output_channels
            .map(NonZeroU32::get)
            .unwrap_or(2) as usize;
        self.erosion = Erosion::new(self.channels, self.sample_rate);
        context.set_latency_samples(self.erosion.latency() as u32);
        true
    }

    fn reset(&mut self) {
        self.erosion.reset();
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        _context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let p = &self.params;
        let shape = p.shape.value();
        let channels = self.channels.min(buffer.channels());
        let open = p.editor_state.is_open();
        #[cfg(feature = "standalone")]
        let sr = self.sample_rate;

        for mut samples in buffer.iter_samples() {
            let settings = Settings {
                freq: p.freq.smoothed.next(),
                amount: p.amount.smoothed.next(),
                blend: p.blend.smoothed.next(),
                width: p.width.smoothed.next(),
                stereo: p.stereo.smoothed.next(),
                drift: p.drift.smoothed.next(),
                shape,
            };

            let mut io = [0.0f32; 2];
            #[cfg(feature = "standalone")]
            let demo = self.demo.as_mut().map(|d| d.next(sr));

            for (c, s) in samples.iter_mut().take(channels).enumerate() {
                #[cfg(feature = "standalone")]
                if let Some(d) = demo {
                    *s = d;
                }
                io[c] = *s;
            }

            // the display shows the input like erosion does
            if open {
                let mono = io[..channels].iter().sum::<f32>() / channels as f32;
                if self.scope.push(mono) {
                    let frame = self.frame_in.input_buffer_mut();
                    self.scope.copy_to(&mut frame.samples);
                    frame.sample_rate = self.sample_rate;
                    self.frame_in.publish();
                }
            }

            self.erosion.process(&mut io[..channels], &settings);

            for (c, s) in samples.iter_mut().take(channels).enumerate() {
                *s = io[c];
            }
        }

        self.live
            .f_eff
            .store(self.erosion.f_eff(), Ordering::Relaxed);

        ProcessStatus::Normal
    }
}

impl Vst3Plugin for Abrasion {
    const VST3_CLASS_ID: [u8; 16] = *b"XeinAbrasionFx01";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Fx, Vst3SubCategory::Distortion];
}

nih_export_vst3!(Abrasion);
