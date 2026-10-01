//! Bounded input conditioning for quiet microphones. No buffering or latency.
pub struct InputGain {
    gain: f32,
}

impl Default for InputGain {
    fn default() -> Self {
        Self { gain: 1.0 }
    }
}

impl InputGain {
    /// Returns the raw input meter (0..1), before automatic gain.
    pub fn process(&mut self, samples: &mut [f32], sample_rate: i32) -> f32 {
        if samples.is_empty() || sample_rate <= 0 {
            return 0.0;
        }
        for sample in samples.iter_mut() {
            if !sample.is_finite() {
                *sample = 0.0;
            }
        }
        let rms = (samples.iter().map(|v| v * v).sum::<f32>() / samples.len() as f32).sqrt();
        // Do not chase digital silence / the noise floor. Cap quiet-input gain
        // at +18 dB; this cannot recover speech buried in office noise.
        let wanted = if rms >= 0.001 {
            (0.06 / rms).clamp(1.0, 8.0)
        } else {
            1.0
        };
        let seconds = samples.len() as f32 / sample_rate as f32;
        let speed = if wanted < self.gain { 0.01 } else { 0.08 };
        let next = self.gain + (wanted - self.gain) * (1.0 - (-seconds / speed).exp());
        let step = (next - self.gain) / samples.len() as f32;
        for sample in samples.iter_mut() {
            self.gain += step;
            *sample = (*sample * self.gain).clamp(-0.98, 0.98);
        }
        self.gain = next;
        if rms < 0.0001 {
            0.0
        } else {
            ((20.0 * rms.log10() + 60.0) / 60.0).clamp(0.0, 1.0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quiet_input_is_lifted_but_silence_and_invalid_samples_stay_silent() {
        let mut gain = InputGain::default();
        let mut quiet = vec![0.004; 1600];
        let meter = gain.process(&mut quiet, 16000);
        assert!(quiet.last().unwrap() > &0.02);
        assert!(meter > 0.1 && meter < 0.4);
        let mut silence = vec![0.0; 1600];
        assert_eq!(gain.process(&mut silence, 16000), 0.0);
        assert!(silence.iter().all(|v| *v == 0.0));
        let mut invalid = [f32::NAN, f32::INFINITY];
        assert_eq!(gain.process(&mut invalid, 16000), 0.0);
        assert_eq!(invalid, [0.0, 0.0]);
    }
    #[test]
    fn gain_is_bounded_and_loud_transients_never_overflow() {
        let mut gain = InputGain::default();
        for _ in 0..20 {
            gain.process(&mut vec![0.001; 1600], 16000);
        }
        assert!(gain.gain <= 8.0);
        let mut loud = vec![0.9; 1600];
        gain.process(&mut loud, 16000);
        assert!(loud.iter().all(|v| v.abs() <= 0.98));
        assert!(gain.gain < 1.01);
    }
}
