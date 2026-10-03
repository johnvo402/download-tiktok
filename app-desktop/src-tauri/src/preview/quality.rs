#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Quality {
    Auto,
    Full,
    Half,
    Quarter,
}
impl Quality {
    pub fn environment() -> Self {
        match std::env::var("JC_PREVIEW_QUALITY").as_deref() {
            Ok("full") => Self::Full,
            Ok("half") => Self::Half,
            Ok("quarter") => Self::Quarter,
            _ => Self::Auto,
        }
    }
    pub fn initial(self, source: (u32, u32), viewport: (f64, f64)) -> f32 {
        match self {
            Self::Full => 1.0,
            Self::Half => 0.5,
            Self::Quarter => 0.25,
            Self::Auto => {
                if source.0.max(source.1) <= 1920 {
                    return 1.0;
                }
                let ratio = (viewport.0 / source.0 as f64).min(viewport.1 / source.1 as f64);
                if ratio <= 0.25 {
                    0.25
                } else if ratio <= 0.5 {
                    0.5
                } else {
                    1.0
                }
            }
        }
    }
    pub fn slower(self, scale: f32, average_ms: f64, frame_budget_ms: f64) -> Option<f32> {
        (self == Self::Auto && scale > 0.25 && average_ms > frame_budget_ms * 1.25)
            .then_some((scale / 2.0).max(0.25))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_and_adaptive_quality_preserve_export() {
        assert_eq!(Quality::Full.initial((3840, 2160), (800.0, 500.0)), 1.0);
        assert_eq!(Quality::Auto.initial((1920, 1080), (800.0, 500.0)), 1.0);
        assert_eq!(Quality::Auto.initial((3840, 2160), (800.0, 500.0)), 0.25);
        assert_eq!(Quality::Auto.slower(1.0, 50.0, 33.33), Some(0.5));
        assert_eq!(Quality::Full.slower(1.0, 50.0, 33.33), None);
        assert_eq!(Quality::Auto.slower(0.25, 50.0, 33.33), None);
    }
}
