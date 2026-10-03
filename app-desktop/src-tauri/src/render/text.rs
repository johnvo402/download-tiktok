//! Shape Unicode using system fonts and cache glyph masks with Swash. Layers are
//! rasterized once on first use, then cached/uploaded and composited by the GPU.
use crate::render_spec::PixelRect;
use cosmic_text::{
    Align, Attrs, Buffer, Color, Family, FontSystem, Metrics, Shaping, SwashCache, Weight, Wrap,
};

#[derive(Clone, Debug)]
pub struct TextStyle {
    pub font: String,
    pub size: u16,
    pub weight: u16,
    pub color: [u8; 4],
    pub outline: [u8; 4],
    pub outline_width: u16,
    pub alignment: String,
    pub vertical: String,
    pub line_height: f32,
    pub padding_x: u32,
    pub padding_y: u32,
    pub background: [u8; 4],
    pub strict_fit: bool,
}
pub struct TextRasterizer {
    fonts: FontSystem,
    glyphs: SwashCache,
}
impl Default for TextRasterizer {
    fn default() -> Self {
        Self {
            fonts: FontSystem::new(),
            glyphs: SwashCache::new(),
        }
    }
}
pub fn rgb(color: &str, opacity: f64) -> Result<[u8; 4], String> {
    if color.len() != 7
        || !color.starts_with('#')
        || !color.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit)
    {
        return Err("Màu lớp chữ không hợp lệ.".into());
    }
    Ok([
        u8::from_str_radix(&color[1..3], 16).unwrap(),
        u8::from_str_radix(&color[3..5], 16).unwrap(),
        u8::from_str_radix(&color[5..7], 16).unwrap(),
        (opacity.clamp(0.0, 1.0) * 255.0).round() as u8,
    ])
}
pub fn ass_rgb(color: &str) -> Result<[u8; 4], String> {
    let code = color.strip_prefix("&H").ok_or("Màu phụ đề không hợp lệ.")?;
    if code.len() != 8 || !code.as_bytes().iter().all(u8::is_ascii_hexdigit) {
        return Err("Màu phụ đề không hợp lệ.".into());
    }
    Ok([
        u8::from_str_radix(&code[6..8], 16).unwrap(),
        u8::from_str_radix(&code[4..6], 16).unwrap(),
        u8::from_str_radix(&code[2..4], 16).unwrap(),
        255 - u8::from_str_radix(&code[..2], 16).unwrap(),
    ])
}
fn over(dst: &mut [u8], rgb: [u8; 4], alpha: u8) {
    let a = alpha as f32 / 255.0;
    let da = dst[3] as f32 / 255.0;
    let total = a + da * (1.0 - a);
    if total <= 0.0 {
        return;
    }
    for channel in 0..3 {
        dst[channel] = ((rgb[channel] as f32 * a + dst[channel] as f32 * da * (1.0 - a)) / total)
            .round() as u8;
    }
    dst[3] = (total * 255.0).round() as u8;
}
impl TextRasterizer {
    pub fn rasterize(
        &mut self,
        text: &str,
        rect: PixelRect,
        style: &TextStyle,
    ) -> Result<Vec<u8>, String> {
        let size = super::frame::byte_len(rect.width, rect.height)?;
        if size > 64 * 1024 * 1024 {
            return Err("Vùng chữ quá lớn để render GPU an toàn.".into());
        }
        let stroke = style.outline_width as u32;
        let px = style.padding_x.max(stroke + 2);
        let py = style.padding_y.max(stroke + 2);
        let width = rect.width.saturating_sub(2 * px).max(1);
        let height = rect.height.saturating_sub(2 * py).max(1);
        let mut buffer = Buffer::new(
            &mut self.fonts,
            Metrics::new(
                style.size as f32,
                style.size as f32 * style.line_height.max(0.8),
            ),
        );
        buffer.set_size(&mut self.fonts, Some(width as f32), None);
        buffer.set_wrap(&mut self.fonts, Wrap::WordOrGlyph);
        // FontSystem uses installed fallback fonts for unsupported glyphs.
        let font = if style.font.trim().is_empty() {
            "Segoe UI"
        } else {
            &style.font
        };
        let alignment = match style.alignment.as_str() {
            "left" => Align::Left,
            "right" => Align::Right,
            _ => Align::Center,
        };
        buffer.set_text(
            &mut self.fonts,
            text,
            &Attrs::new()
                .family(Family::Name(font))
                .weight(Weight(style.weight)),
            Shaping::Advanced,
            Some(alignment),
        );
        buffer.shape_until_scroll(&mut self.fonts, false);
        let total_height = buffer
            .layout_runs()
            .map(|run| run.line_top + run.line_height)
            .fold(0.0_f32, f32::max);
        if style.strict_fit && total_height > height as f32 + 0.5 {
            return Err(
                "Phụ đề không vừa vùng với font thực tế. Hãy mở rộng vùng hoặc giảm cỡ chữ.".into(),
            );
        }
        let y = py as i32
            + match style.vertical.as_str() {
                "top" => 0,
                "bottom" => (height as f32 - total_height).max(0.0) as i32,
                _ => ((height as f32 - total_height) / 2.0).max(0.0) as i32,
            };
        let mut mask = vec![0_u8; (rect.width * rect.height) as usize];
        let mut clipped = false;
        buffer.draw(
            &mut self.fonts,
            &mut self.glyphs,
            Color::rgb(255, 255, 255),
            |gx, gy, w, h, color| {
                for iy in 0..h {
                    for ix in 0..w {
                        let x = gx + px as i32 + ix as i32;
                        let yy = gy + y + iy as i32;
                        if x >= 0 && yy >= 0 && x < rect.width as i32 && yy < rect.height as i32 {
                            mask[(yy as u32 * rect.width + x as u32) as usize] = color.a();
                        } else if color.a() > 0 {
                            clipped = true;
                        }
                    }
                }
            },
        );
        if style.strict_fit && clipped {
            return Err("Một số ký tự vượt vùng phụ đề. Hãy tăng kích thước vùng.".into());
        }
        let mut output = vec![0_u8; size];
        if style.background[3] > 0 {
            // A backing box around the text area, not a fill of the whole blur.
            let end = (y as f32 + total_height + stroke as f32 + 2.0)
                .ceil()
                .min(rect.height as f32) as u32;
            for yy in (y.max(0) as u32).saturating_sub(stroke + 2)..end {
                for xx in px.saturating_sub(stroke + 2)
                    ..rect.width.saturating_sub(px.saturating_sub(stroke + 2))
                {
                    output[((yy * rect.width + xx) * 4) as usize
                        ..((yy * rect.width + xx) * 4 + 4) as usize]
                        .copy_from_slice(&style.background);
                }
            }
        }
        // Dilated outline. Only first-use text does this work.
        let radius = stroke.min(16) as i32;
        for yy in 0..rect.height as i32 {
            for xx in 0..rect.width as i32 {
                let i = (yy as u32 * rect.width + xx as u32) as usize;
                if mask[i] == 0 {
                    continue;
                }
                for dy in -radius..=radius {
                    for dx in -radius..=radius {
                        if dx * dx + dy * dy > radius * radius {
                            continue;
                        }
                        let x = xx + dx;
                        let y = yy + dy;
                        if x >= 0 && y >= 0 && x < rect.width as i32 && y < rect.height as i32 {
                            let at = ((y as u32 * rect.width + x as u32) * 4) as usize;
                            let alpha = mask[i].min(style.outline[3]);
                            if output[at + 3] < alpha {
                                output[at..at + 4].copy_from_slice(&[
                                    style.outline[0],
                                    style.outline[1],
                                    style.outline[2],
                                    alpha,
                                ]);
                            }
                        }
                    }
                }
            }
        }
        for (i, alpha) in mask.iter().copied().enumerate() {
            if alpha > 0 {
                over(
                    &mut output[i * 4..i * 4 + 4],
                    style.color,
                    ((alpha as u16 * style.color[3] as u16) / 255) as u8,
                );
            }
        }
        // Bound glyph memory across long videos containing many scripts/font sizes.
        if self.glyphs.image_cache.len() > 16_384
            || self
                .glyphs
                .image_cache
                .values()
                .flatten()
                .map(|image| image.data.len())
                .sum::<usize>()
                > 32 * 1024 * 1024
        {
            self.glyphs.image_cache.clear();
            self.glyphs.outline_command_cache.clear();
        }
        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn colors_preserve_alpha() {
        assert_eq!(rgb("#ff8040", 0.5).unwrap(), [255, 128, 64, 128]);
        assert_eq!(ass_rgb("&H004080FF").unwrap(), [255, 128, 64, 255]);
    }
    #[test]
    fn unicode_pixels_exist_within_region() {
        let style = TextStyle {
            font: "Segoe UI".into(),
            size: 24,
            weight: 600,
            color: [255; 4],
            outline: [0, 0, 0, 255],
            outline_width: 2,
            alignment: "center".into(),
            vertical: "middle".into(),
            line_height: 1.2,
            padding_x: 5,
            padding_y: 5,
            background: [0; 4],
            strict_fit: true,
        };
        let mut raster = TextRasterizer::default();
        for text in [
            "Cảnh giới Đại Đế",
            "Ma Đế trở về",
            "修仙",
            "日本語",
            "한국어",
        ] {
            let pixels = raster
                .rasterize(
                    text,
                    PixelRect {
                        x: 0,
                        y: 0,
                        width: 320,
                        height: 100,
                    },
                    &style,
                )
                .unwrap();
            assert!(pixels.chunks_exact(4).any(|p| p[3] > 0));
        }
    }
}
