use atomic_float::AtomicF32;
use std::sync::atomic::Ordering;

pub const SCOPE_SIZE: usize = 2048;
pub const SCOPE_HOP: usize = 1024;

// last input samples for the analyzer
#[derive(Clone)]
pub struct ScopeFrame {
    pub samples: [f32; SCOPE_SIZE],
    pub sample_rate: f32,
}

impl Default for ScopeFrame {
    fn default() -> Self {
        Self {
            samples: [0.0; SCOPE_SIZE],
            sample_rate: 48000.0,
        }
    }
}

pub struct Live {
    pub f_eff: AtomicF32,
}

impl Live {
    pub fn new() -> Self {
        Self {
            f_eff: AtomicF32::new(2000.0),
        }
    }

    pub fn f_eff(&self) -> f32 {
        self.f_eff.load(Ordering::Relaxed)
    }
}

// collects samples and publishes a frame every hop
pub struct Scope {
    ring: [f32; SCOPE_SIZE],
    pos: usize,
    since: usize,
}

impl Scope {
    pub fn new() -> Self {
        Self {
            ring: [0.0; SCOPE_SIZE],
            pos: 0,
            since: 0,
        }
    }

    pub fn push(&mut self, x: f32) -> bool {
        self.ring[self.pos] = x;
        self.pos = (self.pos + 1) % SCOPE_SIZE;
        self.since += 1;
        if self.since >= SCOPE_HOP {
            self.since = 0;
            return true;
        }
        false
    }

    // oldest sample first
    pub fn copy_to(&self, out: &mut [f32; SCOPE_SIZE]) {
        let (a, b) = self.ring.split_at(self.pos);
        out[..b.len()].copy_from_slice(b);
        out[b.len()..].copy_from_slice(a);
    }
}
