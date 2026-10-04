use hound::{SampleFormat, WavReader};
use std::{fs::File, io::BufReader, path::Path};

pub type Reader = WavReader<BufReader<File>>;
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Signal {
    pub sample_rate: u32,
    pub channels: u16,
    pub duration: f64,
    pub peak: f64,
    pub rms: f64,
    pub speech_start: Option<f64>,
    pub speech_end: Option<f64>,
}
pub fn open(path: &Path) -> Result<Reader, String> {
    regular_file(path)?;
    WavReader::open(path).map_err(|e| format!("Không đọc được WAV: {e}"))
}
pub fn regular_file(path: &Path) -> Result<(), String> {
    let metadata = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err("Tệp audio không được là liên kết.".into());
        }
    }
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err("Tệp audio không hợp lệ.".into());
    }
    Ok(())
}
pub fn sample(reader: &mut Reader) -> Result<Option<f32>, String> {
    let spec = reader.spec();
    let result = if spec.sample_format == SampleFormat::Float {
        reader.samples::<f32>().next()
    } else {
        reader
            .samples::<i32>()
            .next()
            .map(|r| r.map(|v| v as f32 / (2_f32.powi(spec.bits_per_sample as i32 - 1))))
    };
    result
        .transpose()
        .map_err(|e| format!("WAV bị hỏng: {e}"))?
        .map(|v| {
            if v.is_finite() {
                Ok(v)
            } else {
                Err("WAV chứa mẫu âm thanh không hợp lệ.".into())
            }
        })
        .transpose()
}
pub fn inspect(path: &Path) -> Result<Signal, String> {
    let mut reader = open(path)?;
    let spec = reader.spec();
    if spec.channels == 0 || spec.sample_rate == 0 || reader.duration() == 0 {
        return Err("WAV không chứa âm thanh.".into());
    }
    let mut result = Signal {
        sample_rate: spec.sample_rate,
        channels: spec.channels,
        duration: reader.duration() as f64 / spec.sample_rate as f64,
        peak: 0.0,
        rms: 0.0,
        speech_start: None,
        speech_end: None,
    };
    let mut count = 0_u64;
    let mut square = 0.0;
    while let Some(v) = sample(&mut reader)? {
        let value = f64::from(v.abs());
        result.peak = result.peak.max(value);
        square += f64::from(v).powi(2);
        if value > 0.001 {
            // Diagnostic only. Never used to crop speech.
            let t = count as f64 / f64::from(spec.sample_rate * u32::from(spec.channels));
            result.speech_start.get_or_insert(t);
            result.speech_end = Some(t);
        }
        count += 1;
    }
    let expected =
        (result.duration * spec.sample_rate as f64).round() as u64 * spec.channels as u64;
    if count != expected {
        return Err("WAV bị thiếu dữ liệu âm thanh.".into());
    }
    result.rms = (square / count.max(1) as f64).sqrt();
    Ok(result)
}
pub fn validate_stem(path: &Path, duration: f64) -> Result<Signal, String> {
    let signal = inspect(path)?;
    if signal.sample_rate != super::RATE
        || signal.channels != 2
        || (signal.duration - duration).abs() > 0.15
    {
        return Err("Audio tách thoại không khớp video. Hãy tạo lại audio tách thoại.".into());
    }
    Ok(signal)
}
