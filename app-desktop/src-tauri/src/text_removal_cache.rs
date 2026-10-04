//! Seekable, bounded RGBA span cache. Legacy numbered PNG sequences remain readable.
use serde::Deserialize;
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

#[derive(Clone, Deserialize)]
pub struct Entry {
    pub pts: f64,
    pub end: f64,
    pub offset: u64,
    pub length: u64,
}
#[derive(Deserialize)]
pub struct Metadata {
    version: u32,
    complete: bool,
    pub width: u32,
    pub height: u32,
    bytes: u64,
    pub frames: Vec<Entry>,
}
pub struct Reader {
    file: File,
    pub metadata: Metadata,
    last: Option<usize>,
    pixels: Vec<u8>,
}
impl Reader {
    pub fn open(path: &Path) -> Result<Self, String> {
        crate::audio::signal::regular_file(path)?;
        crate::audio::signal::regular_file(&path.with_extension("json"))?;
        let metadata: Metadata = serde_json::from_reader(
            File::open(path.with_extension("json")).map_err(|e| e.to_string())?,
        )
        .map_err(|_| "Thông tin cache xóa chữ bị hỏng.")?;
        let mut file = File::open(path).map_err(|e| e.to_string())?;
        let mut magic = [0; 8];
        file.read_exact(&mut magic)
            .map_err(|_| "Cache xóa chữ thiếu dữ liệu.")?;
        let size = file.metadata().map_err(|e| e.to_string())?.len();
        let raw = crate::render::frame::byte_len(metadata.width, metadata.height)?;
        if magic != *b"JCPATCH2"
            || metadata.version != 2
            || !metadata.complete
            || size != metadata.bytes
            || raw > 256 * 1024 * 1024
            || metadata.frames.is_empty()
        {
            return Err("Cache phục hồi chưa hoàn tất hoặc không hợp lệ.".into());
        }
        let mut previous = 8;
        let mut end = -1.0;
        for f in &metadata.frames {
            if !f.pts.is_finite()
                || !f.end.is_finite()
                || f.pts < 0.0
                || f.end <= f.pts
                || f.pts < end - 1e-7
                || f.offset != previous
                || f.length == 0
                || f.length > raw as u64 + 65536
                || f.offset.checked_add(f.length).is_none_or(|n| n > size)
            {
                return Err("Index frame phục hồi không hợp lệ.".into());
            }
            previous = f.offset + f.length;
            end = f.end;
        }
        if previous != size {
            return Err("Cache phục hồi còn dữ liệu không được xác nhận.".into());
        }
        Ok(Self {
            file,
            metadata,
            last: None,
            pixels: Vec::new(),
        })
    }
    pub fn frame(&mut self, time: f64) -> Result<Option<&[u8]>, String> {
        let i = self
            .metadata
            .frames
            .partition_point(|f| f.pts <= time + 1e-7);
        if i == 0 || time >= self.metadata.frames[i - 1].end - 1e-7 {
            self.last = None;
            self.pixels = Vec::new(); // Do not retain one full ROI buffer per inactive span.
            return Ok(None);
        }
        let i = i - 1;
        if self.last != Some(i) {
            let f = &self.metadata.frames[i];
            self.file
                .seek(SeekFrom::Start(f.offset))
                .map_err(|e| e.to_string())?;
            let compressed = (&mut self.file).take(f.length);
            let decoder = flate2::read::ZlibDecoder::new(compressed);
            let expected =
                crate::render::frame::byte_len(self.metadata.width, self.metadata.height)?;
            self.pixels.clear();
            decoder
                .take(expected as u64 + 1)
                .read_to_end(&mut self.pixels)
                .map_err(|_| "Pixels cache phục hồi bị hỏng.")?;
            if self.pixels.len() != expected {
                return Err("Frame cache phục hồi thiếu hoặc vượt kích thước.".into());
            }
            self.last = Some(i);
        }
        Ok(Some(&self.pixels))
    }
    pub fn verify(&mut self) -> Result<(), String> {
        for i in 0..self.metadata.frames.len() {
            let pts = self.metadata.frames[i].pts;
            self.frame(pts)?;
        }
        self.last = None;
        self.pixels = Vec::new();
        Ok(())
    }
}

pub enum PatchReader {
    Packed(Reader),
    Legacy {
        pattern: PathBuf,
        start: f64,
        end: f64,
        fps: f64,
        last: Option<PathBuf>,
        pixels: Vec<u8>,
    },
}
impl PatchReader {
    pub fn open(p: &crate::text_removal_service::AiTextRemovalPatch) -> Result<Self, String> {
        if p.image_pattern.extension().is_some_and(|e| e == "pack") {
            let r = Reader::open(&p.image_pattern)?;
            if (r.metadata.width, r.metadata.height) != (p.rect.width, p.rect.height) {
                return Err("Kích thước cache không khớp vùng phục hồi.".into());
            }
            Ok(Self::Packed(r))
        } else {
            Ok(Self::Legacy {
                pattern: p.image_pattern.clone(),
                start: p.start_seconds,
                end: p.end_seconds,
                fps: p.fps,
                last: None,
                pixels: Vec::new(),
            })
        }
    }
    pub fn frame(&mut self, pts: f64) -> Result<Option<&[u8]>, String> {
        match self {
            Self::Packed(r) => r.frame(pts),
            Self::Legacy {
                pattern,
                start,
                end,
                fps,
                last,
                pixels,
            } => {
                if pts < *start || pts >= *end {
                    return Ok(None);
                }
                let i = ((pts - *start) * *fps + 0.0001).floor() as u64 + 1;
                let path = PathBuf::from(
                    pattern
                        .to_string_lossy()
                        .replace("%08d", &format!("{i:08}")),
                );
                if last.as_ref() != Some(&path) {
                    *pixels = image::open(&path)
                        .map_err(|_| "Thiếu frame cache phục hồi cũ.")?
                        .into_rgba8()
                        .into_raw();
                    *last = Some(path);
                }
                Ok(Some(pixels))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    #[test]
    fn vfr_frames_do_not_leak_into_gaps_and_corrupt_data_fails() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("span.pack");
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
        z.write_all(&[1, 2, 3, 255]).unwrap();
        let bytes = z.finish().unwrap();
        std::fs::write(&path, [b"JCPATCH2".as_slice(), &bytes].concat()).unwrap();
        std::fs::write(path.with_extension("json"),serde_json::to_vec(&serde_json::json!({"version":2,"complete":true,"width":1,"height":1,"bytes":8+bytes.len(),"frames":[{"pts":1.123456,"end":1.175,"offset":8,"length":bytes.len()}]})).unwrap()).unwrap();
        let mut reader = Reader::open(&path).unwrap();
        assert_eq!(reader.frame(1.14).unwrap(), Some(&[1, 2, 3, 255][..]));
        assert!(reader.frame(1.176).unwrap().is_none());
        assert!(reader.frame(1.12).unwrap().is_none());
        std::fs::write(
            &path,
            [b"JCPATCH2".as_slice(), &vec![0; bytes.len()]].concat(),
        )
        .unwrap();
        let mut reader = Reader::open(&path).unwrap();
        assert!(reader.frame(1.14).is_err());
    }
}
