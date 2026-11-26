//! The pill's rolling waveform, copied from PillApp.tsx (lines 44-82 and 198-262),
//! and a fake microphone feed at the real rate (one level every 40 ms).

pub const BAR_COUNT: usize = 15;
pub const BAR_MIN_SCALE: f32 = 0.14;
pub const BAR_ATTACK: f32 = 0.45;
pub const BAR_DECAY: f32 = 0.16;
pub const BAR_EDGE_GAIN: f32 = 0.3;
pub const RMS_CEILING: f32 = 0.07;
/// The real `audio://level` cadence.
pub const FEED_MS: u64 = 40;

/// Raised-cosine envelope: 1.0 at the centre, BAR_EDGE_GAIN at both ends.
pub fn envelope() -> [f32; BAR_COUNT] {
    let mut out = [1.0; BAR_COUNT];
    if BAR_COUNT <= 1 {
        return out;
    }
    for (i, v) in out.iter_mut().enumerate() {
        let hann =
            0.5 - 0.5 * ((2.0 * std::f32::consts::PI * i as f32) / (BAR_COUNT as f32 - 1.0)).cos();
        *v = BAR_EDGE_GAIN + (1.0 - BAR_EDGE_GAIN) * hann;
    }
    out
}

/// Raw RMS to a 0..1 perceptual level.
pub fn perceptual_level(rms: f32) -> f32 {
    (rms.max(0.0) / RMS_CEILING).min(1.0).sqrt()
}

pub struct Wave {
    history: Vec<f32>,
    display: [f32; BAR_COUNT],
    env: [f32; BAR_COUNT],
}

impl Default for Wave {
    fn default() -> Self {
        Self {
            history: vec![0.0; BAR_COUNT],
            display: [0.0; BAR_COUNT],
            env: envelope(),
        }
    }
}

impl Wave {
    /// A new level enters on the right; the oldest leaves on the left.
    pub fn push_rms(&mut self, rms: f32) {
        self.history.push(perceptual_level(rms));
        if self.history.len() > BAR_COUNT {
            self.history.remove(0);
        }
    }

    /// One animation frame: ease every bar toward its target. Returns the scales.
    pub fn tick(&mut self) -> [f32; BAR_COUNT] {
        for i in 0..BAR_COUNT {
            let target = BAR_MIN_SCALE
                + self.history.get(i).copied().unwrap_or(0.0) * self.env[i] * (1.0 - BAR_MIN_SCALE);
            let current = self.display[i];
            let rate = if target > current {
                BAR_ATTACK
            } else {
                BAR_DECAY
            };
            self.display[i] = current + (target - current) * rate;
        }
        // Same rounding as `toFixed(3)`.
        self.display.map(|v| (v * 1000.0).round() / 1000.0)
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// Speech-like RMS: syllables of 120-280 ms with short pauses, plus noise.
pub struct FakeMic {
    seed: u32,
    t: u32,
    syllable_left: u32,
    level: f32,
}

impl Default for FakeMic {
    fn default() -> Self {
        Self {
            seed: 0x5eed_1234,
            t: 0,
            syllable_left: 0,
            level: 0.0,
        }
    }
}

impl FakeMic {
    fn rand(&mut self) -> f32 {
        // xorshift32
        let mut x = self.seed;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.seed = x;
        (x as f32) / (u32::MAX as f32)
    }

    /// The next 40 ms RMS reading.
    pub fn next_rms(&mut self) -> f32 {
        self.t += 1;
        if self.syllable_left == 0 {
            // Every so often a pause, otherwise a new syllable.
            let pause = self.rand() < 0.18;
            self.level = if pause {
                0.004
            } else {
                0.03 + self.rand() * 0.11
            };
            self.syllable_left = 3 + (self.rand() * 5.0) as u32;
        }
        self.syllable_left -= 1;
        let noise = 0.8 + self.rand() * 0.4;
        self.level * noise
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_matches_the_react_numbers() {
        let e = envelope();
        assert!((e[0] - 0.3).abs() < 1e-6);
        assert!((e[7] - 1.0).abs() < 1e-6);
        assert!((e[14] - 0.3).abs() < 1e-6);
        assert!((e[3] - e[11]).abs() < 1e-6);
    }

    #[test]
    fn silence_rests_on_the_min_scale_and_loudness_attacks_fast() {
        let mut w = Wave::default();
        let mut s = [0.0; BAR_COUNT];
        for _ in 0..120 {
            s = w.tick();
        }
        assert!(s.iter().all(|v| (v - BAR_MIN_SCALE).abs() < 0.002));
        w.push_rms(1.0);
        let after = w.tick();
        // The newest sample sits at the right end; one frame moves 45% of the way.
        let target = BAR_MIN_SCALE + envelope()[14] * (1.0 - BAR_MIN_SCALE);
        let expect = BAR_MIN_SCALE + (target - BAR_MIN_SCALE) * BAR_ATTACK;
        assert!(
            (after[14] - expect).abs() < 0.002,
            "{} vs {}",
            after[14],
            expect
        );
    }

    #[test]
    fn perceptual_curve_is_clamped() {
        assert_eq!(perceptual_level(-1.0), 0.0);
        assert_eq!(perceptual_level(1.0), 1.0);
        assert!((perceptual_level(0.07 / 4.0) - 0.5).abs() < 1e-6);
    }
}
