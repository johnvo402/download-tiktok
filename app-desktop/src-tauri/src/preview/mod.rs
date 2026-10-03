pub mod clock;
pub mod commands;
mod manager;
mod quality;
pub(crate) mod scene;
mod software;
#[cfg(windows)]
mod surface;
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Bounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub dpr: f64,
    pub visible: bool,
}
impl Bounds {
    pub fn validate(&self) -> Result<(), String> {
        if ![self.x, self.y, self.width, self.height, self.dpr]
            .iter()
            .all(|v| v.is_finite())
            || !(0.5..=4.0).contains(&self.dpr)
            || self.width <= 0.0
            || self.height <= 0.0
            || self.width * self.dpr > 16384.0
            || self.height * self.dpr > 16384.0
            || self.x.abs() > 32768.0
            || self.y.abs() > 32768.0
        {
            return Err("Kích thước preview không hợp lệ.".into());
        }
        Ok(())
    }
}
pub use manager::PreviewManager;
#[cfg(test)]
mod tests;
