//! FFmpeg inputs are passed as arguments; generated filter/list files contain no user paths.
use super::{asset_id, caption_states, seconds, Plan};
use crate::{
    render::text::{rgb, TextRasterizer, TextStyle},
    render_spec::PixelRect,
    timeline::{Clip, ClipContent, ClipStyle, Fit},
};
use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping, Weight, Wrap};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub struct Graph {
    pub inputs: Vec<Vec<std::ffi::OsString>>,
    pub filters: String,
    pub audio: bool,
    pub copy_video: bool,
}
fn num(n: f64) -> String {
    format!("{n:.9}")
}
fn input(path: &Path, before: &[String]) -> Vec<std::ffi::OsString> {
    let mut args: Vec<_> = before.iter().map(Into::into).collect();
    args.push("-i".into());
    args.push(ffmpeg_path(path).into_os_string());
    args
}
/// FFmpeg's concat URL resolver misinterprets Windows verbatim paths when joining relative PNGs.
pub fn ffmpeg_path(path: &Path) -> PathBuf {
    let value = path.to_string_lossy();
    if let Some(rest) = value.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    if let Some(rest) = value.strip_prefix(r"\\?\") {
        return PathBuf::from(rest);
    }
    path.to_path_buf()
}
fn size(v: f64) -> u32 {
    v.round().max(1.0) as u32
}
pub fn build(
    plan: &Plan,
    root: &Path,
    master: Option<&Path>,
    cancelled: &impl Fn() -> bool,
) -> Result<Graph, String> {
    let (w, h) = (plan.document.settings.width, plan.document.settings.height);
    if w as u64 * h as u64 > 32_000_000 {
        return Err("Canvas quá lớn để export; hãy giảm kích thước khung hình.".into());
    }
    let fps = format!(
        "{}/{}",
        plan.options.frame_rate.numerator, plan.options.frame_rate.denominator
    );
    let d = num(plan.duration());
    let mut graph = Graph {
        inputs: vec![],
        filters: format!(
            "color=c={}:s={w}x{h}:r={fps}:d={d},format=rgba[base0];\n",
            plan.document.settings.background_color
        ),
        audio: master.is_some(),
        copy_video: false,
    };
    let mut raster = TextRasterizer::default();
    let mut fonts = FontSystem::new();
    for (layer, c) in plan.visual_clips().into_iter().enumerate() {
        if cancelled() {
            return Err("Đã hủy export.".into());
        }
        let (from, to) = plan.window(c)?.unwrap();
        let duration = seconds(to - from);
        let offset = seconds(from - plan.start);
        let rate = c.rate.numerator as f64 / c.rate.denominator as f64;
        let idx = graph.inputs.len();
        let text = matches!(
            c.content,
            ClipContent::Text { .. } | ClipContent::Subtitles { .. }
        );
        if text {
            let path = text_input(plan, c, root, layer, &mut raster, &mut fonts, cancelled)?;
            graph.inputs.push(input(
                &path,
                &["-f".into(), "concat".into(), "-safe".into(), "0".into()],
            ));
        } else {
            let path = &plan.assets[asset_id(c).unwrap()];
            let still = matches!(c.content, ClipContent::Image { .. });
            let args = if still {
                vec![
                    "-loop".into(),
                    "1".into(),
                    "-framerate".into(),
                    fps.clone(),
                    "-t".into(),
                    num(duration),
                ]
            } else {
                vec![
                    "-noaccurate_seek".into(),
                    "-ss".into(),
                    num(seconds(c.timeline_to_source_us(from)?.unwrap())),
                    "-threads".into(),
                    "1".into(),
                ]
            };
            graph.inputs.push(input(path, &args));
        }
        let t = &c.transform;
        let crop = &t.crop;
        let mut chain = if text || matches!(c.content, ClipContent::Image { .. }) {
            format!("[{idx}:v]setpts=PTS-STARTPTS,fps={fps},format=rgba")
        } else {
            format!(
                // Retain the keyframe preroll and signed PTS after input -ss.
                // fps holds the prior VFR frame until its successor's timestamp;
                // STARTPTS would pull that future frame to the cut/batch start.
                "[{idx}:v]setpts=PTS/{},fps={fps}:start_time=0:round=up,format=rgba",
                num(rate)
            )
        };
        if text {
            // Text clip-path is relative to the full canvas, without refitting the glyphs.
            chain.push_str(&format!(
                ",crop={}:{}:{}:{}:exact=1,pad={w}:{h}:{}:{}:color=black@0",
                size(w as f64 * crop.width),
                size(h as f64 * crop.height),
                size_zero(w as f64 * crop.x),
                size_zero(h as f64 * crop.y),
                size_zero(w as f64 * crop.x),
                size_zero(h as f64 * crop.y)
            ));
        } else {
            let asset = plan
                .document
                .assets
                .iter()
                .find(|a| Some(a.id.as_str()) == asset_id(c))
                .unwrap();
            let sw = asset.metadata.width.unwrap_or(w) as f64 * crop.width;
            let sh = asset.metadata.height.unwrap_or(h) as f64 * crop.height;
            let fit = if t.fit == Fit::Cover {
                (w as f64 / sw).max(h as f64 / sh)
            } else {
                (w as f64 / sw).min(h as f64 / sh)
            };
            let (pw, ph) = if t.fit == Fit::Stretch {
                (w, h)
            } else {
                (size(sw * fit), size(sh * fit))
            };
            chain.push_str(&format!(",crop=w='max(1,iw*{})':h='max(1,ih*{})':x=iw*{}:y=ih*{}:exact=1,scale={pw}:{ph}:flags=bicubic,setsar=1",
                num(crop.width),num(crop.height),num(crop.x),num(crop.y)));
            // Crop/pad before the affine transform matches overflow:hidden on preview's layer.
            chain.push_str(&format!(
                ",crop={}:{}:(iw-ow)/2:(ih-oh)/2,pad={w}:{h}:(ow-iw)/2:(oh-ih)/2:color=black@0",
                pw.min(w),
                ph.min(h)
            ));
        }
        let local = format!("t+{}", num(seconds(from - c.start_us)));
        let geq_local = format!("T+{}", num(seconds(from - c.start_us)));
        let animated = c.effects.as_ref().is_some_and(|e| !e.keyframes.is_empty());
        let alpha = crate::timeline_effects::alpha_expression(c, &plan.document, &geq_local)?;
        let has_alpha = c
            .effects
            .as_ref()
            .is_some_and(|e| !e.keyframes.is_empty() || e.fade_in_us > 0 || e.fade_out_us > 0)
            || c.transition_in.is_some()
            || c.linked_clip_id.is_some()
                && crate::timeline_effects::audio_ramps(c, &plan.document)? != (None, None);
        if has_alpha {
            chain.push_str(&format!(
                ",geq=r='r(X,Y)':g='g(X,Y)':b='b(X,Y)':a='alpha(X,Y)*({alpha})',format=rgba"
            ));
        } else {
            chain.push_str(&format!(",colorchannelmixer=aa={}", num(t.opacity)));
        }
        let keys = c
            .effects
            .as_ref()
            .map(|e| e.keyframes.as_slice())
            .unwrap_or(&[]);
        let max_x = if animated {
            keys.iter().map(|k| k.scale_x).fold(0f64, f64::max)
        } else {
            t.scale_x
        };
        let max_y = if animated {
            keys.iter().map(|k| k.scale_y).fold(0f64, f64::max)
        } else {
            t.scale_y
        };
        let tw = size(w as f64 * max_x);
        let th = size(h as f64 * max_y);
        let angle = t.rotation.to_radians();
        let rw = size((tw as f64 * angle.cos().abs() + th as f64 * angle.sin().abs()).ceil());
        let rh = size((tw as f64 * angle.sin().abs() + th as f64 * angle.cos().abs()).ceil());
        if rw as u64 * rh as u64 > 32_000_000 {
            return Err(format!("Lớp {} quá lớn để export. Hãy giảm scale.", c.id));
        }
        if animated {
            let sx = crate::timeline_effects::motion_expression(c, 2, &local);
            let sy = crate::timeline_effects::motion_expression(c, 3, &local);
            chain.push_str(&format!(",scale=w='max(1,floor({w}*({sx})+0.5))':h='max(1,floor({h}*({sy})+0.5))':eval=frame:flags=bicubic,pad={tw}:{th}:(ow-iw)/2:(oh-ih)/2:color=black@0:eval=frame"));
        } else {
            chain.push_str(&format!(",scale={tw}:{th}:flags=bicubic"));
        }
        if t.flip_x {
            chain.push_str(",hflip");
        }
        if t.flip_y {
            chain.push_str(",vflip");
        }
        if t.rotation != 0.0 {
            chain.push_str(&format!(",rotate={}:ow={rw}:oh={rh}:c=black@0", num(angle)));
        }
        chain.push_str(&format!(
            ",trim=duration={},setpts=PTS-STARTPTS+{}/TB[layer{layer}];\n",
            num(duration),
            num(offset)
        ));
        graph.filters.push_str(&chain);
        let clock = format!(
            "t+{}-{}",
            num(seconds(plan.start)),
            num(seconds(c.start_us))
        );
        let x = crate::timeline_effects::motion_expression(c, 0, &clock);
        let y = crate::timeline_effects::motion_expression(c, 1, &clock);
        graph.filters.push_str(&format!("[base{layer}][layer{layer}]overlay=x='({x})*{w}-overlay_w/2':y='({y})*{h}-overlay_h/2':eof_action=pass:repeatlast=0:format=auto:enable='gte(t,{})*lt(t,{})'[base{}];\n",
            num(offset),num(seconds(to-plan.start)),layer+1));
    }
    graph.filters.push_str(&format!(
        "[base{}]scale={}:{},setsar=1,format=yuv420p[vout];\n",
        plan.visual_clips().len(),
        plan.options.width,
        plan.options.height
    ));
    if let Some(master) = master {
        let idx = graph.inputs.len();
        graph.inputs.push(input(master, &[]));
        graph.filters.push_str(&format!(
            "[{idx}:a:0]atrim=duration={d},asetpts=PTS-STARTPTS[aout]\n"
        ));
    } else {
        graph.filters = graph.filters.trim_end().trim_end_matches(';').to_string();
    }
    Ok(graph)
}
fn size_zero(v: f64) -> u32 {
    v.round().max(0.0) as u32
}
fn typography(plan: &Plan, c: &Clip) -> ClipStyle {
    if let Some(s) = &c.style {
        return s.clone();
    }
    let s = if let ClipContent::Subtitles { video_content_id } = &c.content {
        plan.document
            .video_contents
            .iter()
            .find(|v| v.id == *video_content_id)
            .map(|v| &v.settings.subtitle)
    } else {
        None
    };
    ClipStyle {
        font_family: s.map_or("Arial".into(), |s| s.font_family.clone()),
        font_size: s.map_or(48.0, |s| s.font_size as f64),
        font_weight: s.map_or(700, |s| s.font_weight as u32),
        color: s.map_or("#FFFFFF".into(), |s| s.text_color.clone()),
        background_color: s.map_or("#000000".into(), |s| s.background_color.clone()),
        background_opacity: s.map_or(0.5, |s| {
            if s.background_enabled {
                s.background_opacity
            } else {
                0.0
            }
        }),
        alignment: s.map_or("center".into(), |s| s.alignment.clone()),
    }
}
fn text_input(
    plan: &Plan,
    c: &Clip,
    root: &Path,
    n: usize,
    raster: &mut TextRasterizer,
    fonts: &mut FontSystem,
    cancelled: &impl Fn() -> bool,
) -> Result<PathBuf, String> {
    let s = typography(plan, c);
    let (w, h) = (plan.document.settings.width, plan.document.settings.height);
    let mut list = String::new();
    let mut cache = std::collections::HashMap::<String, String>::new();
    let mut last = String::new();
    for (a, b, text) in caption_states(plan, c)? {
        if cancelled() {
            return Err("Đã hủy export.".into());
        }
        let filename = if let Some(name) = cache.get(&text) {
            name.clone()
        } else {
            let name = format!("text-{n}-{}.png", cache.len());
            let mut canvas = image::RgbaImage::new(w, h);
            if !text.trim().is_empty() {
                // Measure the same 90% wrapping area as the preview's centered inline box.
                let max_width = (w as f32 * 0.9 - 16.0).max(1.0);
                let mut measure = Buffer::new(
                    fonts,
                    Metrics::new(s.font_size as f32, s.font_size as f32 * 1.25),
                );
                measure.set_size(fonts, Some(max_width), None);
                measure.set_wrap(fonts, Wrap::WordOrGlyph);
                measure.set_text(
                    fonts,
                    &text,
                    &Attrs::new()
                        .family(Family::Name(&s.font_family))
                        .weight(Weight(s.font_weight as u16)),
                    Shaping::Advanced,
                    None,
                );
                measure.shape_until_scroll(fonts, false);
                let width = measure
                    .layout_runs()
                    .map(|r| r.line_w)
                    .fold(0f32, f32::max)
                    .ceil() as u32
                    + 16;
                let height = measure
                    .layout_runs()
                    .map(|r| r.line_top + r.line_height)
                    .fold(0f32, f32::max)
                    .ceil() as u32
                    + 6;
                let bw = width.min(w).max(1);
                let bh = height.min(h).max(1);
                let rgba = raster.rasterize(
                    &text,
                    PixelRect {
                        x: 0,
                        y: 0,
                        width: bw,
                        height: bh,
                    },
                    &TextStyle {
                        font: s.font_family.clone(),
                        size: s.font_size.round() as u16,
                        weight: s.font_weight as u16,
                        color: rgb(&s.color, 1.0)?,
                        outline: [0; 4],
                        outline_width: 0,
                        alignment: s.alignment.clone(),
                        vertical: "center".into(),
                        line_height: 1.25,
                        padding_x: 8,
                        padding_y: 3,
                        background: [0; 4],
                        strict_fit: false,
                    },
                )?;
                let pixels =
                    image::RgbaImage::from_raw(bw, bh, rgba).ok_or("Không tạo được lớp chữ.")?;
                let mut text_box = image::RgbaImage::from_pixel(
                    bw,
                    bh,
                    image::Rgba(rgb(&s.background_color, s.background_opacity)?),
                );
                image::imageops::overlay(&mut text_box, &pixels, 0, 0);
                image::imageops::overlay(
                    &mut canvas,
                    &text_box,
                    ((w - bw) / 2) as i64,
                    ((h - bh) / 2) as i64,
                );
            }
            canvas.save(root.join(&name)).map_err(|e| e.to_string())?;
            cache.insert(text, name.clone());
            name
        };
        // image2 timebase is explicitly microseconds, so captions aren't rounded to its default 25 fps.
        list.push_str(&format!(
            "file '{filename}'\noption framerate 1000000\nduration {}\n",
            num(seconds(b - a))
        ));
        last = filename;
    }
    list.push_str(&format!("file '{last}'\noption framerate 1000000\n"));
    let path = root.join(format!("text-{n}.ffconcat"));
    fs::write(&path, list).map_err(|e| e.to_string())?;
    Ok(path)
}
