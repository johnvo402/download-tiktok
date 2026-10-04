pub mod commands;
pub mod mixer;
pub mod processing;
pub mod signal;
#[cfg(test)]
mod tests;
pub mod timeline;
pub mod transport;

pub const RATE: u32 = 48_000;
pub const BLOCK: usize = 1024;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct ProcessingSettings {
    pub head_padding_ms: u32,
    pub tail_padding_ms: u32,
    pub max_fit_speed: f64,
    pub normalize: bool,
    pub duck_attack_ms: u32,
    pub duck_release_ms: u32,
}
impl Default for ProcessingSettings {
    fn default() -> Self {
        Self {
            head_padding_ms: 100,
            tail_padding_ms: 60,
            max_fit_speed: 1.2,
            normalize: true,
            duck_attack_ms: 60,
            duck_release_ms: 180,
        }
    }
}
impl ProcessingSettings {
    pub fn validate(&self) -> Result<(), String> {
        if self.head_padding_ms > 300
            || self.tail_padding_ms > 300
            || !self.max_fit_speed.is_finite()
            || !(1.0..=2.0).contains(&self.max_fit_speed)
            || self.duck_attack_ms > 500
            || self.duck_release_ms > 2000
        {
            return Err("Thông số xử lý âm thanh không hợp lệ.".into());
        }
        Ok(())
    }
}
