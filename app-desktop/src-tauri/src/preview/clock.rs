use std::time::Instant;
/// Monotonic media clock; audio anchors correct drift without a frame-index clock.
pub struct Clock {
    anchor: Instant,
    position: f64,
    pub playing: bool,
    duration: f64,
}
impl Clock {
    pub fn new(duration: f64) -> Self {
        Self {
            anchor: Instant::now(),
            position: 0.0,
            playing: false,
            duration,
        }
    }
    pub fn now(&self) -> f64 {
        self.at(Instant::now())
    }
    fn at(&self, now: Instant) -> f64 {
        (self.position
            + if self.playing {
                now.duration_since(self.anchor).as_secs_f64()
            } else {
                0.0
            })
        .clamp(0.0, self.duration)
    }
    pub fn seek(&mut self, position: f64) -> Result<(), String> {
        if !position.is_finite() || position < 0.0 || position > self.duration + 0.001 {
            return Err("Vị trí phát không hợp lệ.".into());
        }
        self.position = position.min(self.duration);
        self.anchor = Instant::now();
        Ok(())
    }
    pub fn set_playing(&mut self, playing: bool) {
        self.position = self.now();
        self.anchor = Instant::now();
        self.playing = playing;
    }
    pub fn audio_anchor(&mut self, time: f64) {
        if time.is_finite()
            && time >= 0.0
            && time <= self.duration
            && (self.now() - time).abs() > 0.025
        {
            let _ = self.seek(time);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn play_pause_seek_audio() {
        let mut c = Clock::new(4000.0);
        c.seek(3600.1).unwrap();
        assert_eq!(c.now(), 3600.1);
        c.set_playing(true);
        assert!((c.at(c.anchor + std::time::Duration::from_millis(500)) - 3600.6).abs() < 1e-8);
        c.set_playing(false);
        let p = c.now();
        assert_eq!(c.now(), p);
        c.audio_anchor(42.123);
        assert_eq!(c.now(), 42.123);
        assert!(c.seek(f64::NAN).is_err());
        assert!(c.seek(5000.0).is_err());
    }
}
