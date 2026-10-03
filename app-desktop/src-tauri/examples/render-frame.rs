//! cargo run --manifest-path src-tauri/Cargo.toml --example render-frame -- input.png output.png
use ai_video_localization_lib::render::{
    frame::{PixelFormat, VideoFrame},
    graph::RenderScene,
    renderer::Renderer,
};
fn main() -> Result<(), String> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 3 {
        return Err("Usage: render-frame input.png output.png".into());
    }
    let input = image::open(&args[1])
        .map_err(|e| e.to_string())?
        .into_rgba8();
    let mut renderer = Renderer::new(RenderScene::empty(input.width(), input.height()), &[])?;
    let mut output = vec![0; input.len()];
    renderer.render_frame(
        VideoFrame {
            width: input.width(),
            height: input.height(),
            pts_seconds: 0.0,
            format: PixelFormat::Rgba8,
            data: &input,
        },
        &mut output,
    )?;
    image::save_buffer(
        &args[2],
        &output,
        input.width(),
        input.height(),
        image::ColorType::Rgba8,
    )
    .map_err(|e| e.to_string())?;
    println!("Rendered on {}", renderer.adapter_name());
    Ok(())
}
