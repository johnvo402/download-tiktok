#[cfg(windows)]
fn main() {
    use ai_video_localization_lib::{
        hardware_acceleration::VideoCodec, media_gpu::windows::NativePipeline,
        render::graph::RenderScene,
    };
    struct Logger;
    impl log::Log for Logger {
        fn enabled(&self, _: &log::Metadata) -> bool {
            true
        }
        fn log(&self, r: &log::Record) {
            if r.level() <= log::Level::Warn || r.target().starts_with("ai_video") {
                eprintln!("{} {}", r.level(), r.args());
            }
        }
        fn flush(&self) {}
    }
    static LOGGER: Logger = Logger;
    log::set_logger(&LOGGER).unwrap();
    log::set_max_level(log::LevelFilter::Info);
    let input = std::path::PathBuf::from(std::env::args_os().nth(1).expect("input video"));
    let output = std::path::PathBuf::from(std::env::args_os().nth(2).expect("output video"));
    let width = std::env::args()
        .nth(3)
        .unwrap_or("320".into())
        .parse()
        .unwrap();
    let height = std::env::args()
        .nth(4)
        .unwrap_or("240".into())
        .parse()
        .unwrap();
    let codec = if std::env::args().nth(5).as_deref() == Some("hevc") {
        VideoCodec::Hevc
    } else {
        VideoCodec::H264
    };
    let input = std::path::absolute(input).unwrap();
    let output = std::path::absolute(output).unwrap();
    let mut pipeline = NativePipeline::new(
        &input,
        &output,
        RenderScene::empty(width, height),
        &[],
        codec,
        4_000_000,
        None,
    )
    .unwrap();
    let start = std::time::Instant::now();
    let counters = pipeline
        .run(&std::sync::atomic::AtomicU8::new(0), |_, _| {})
        .unwrap();
    println!(
        "Native hardware {codec:?}: {counters:?}; wall={:?}",
        start.elapsed()
    );
}
#[cfg(not(windows))]
fn main() {}
