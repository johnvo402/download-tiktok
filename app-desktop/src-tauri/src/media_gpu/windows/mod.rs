//! Audited Windows COM/D3D boundary; no raw pointer escapes these modules.
mod decoder;
mod device;
mod encoder;
mod interop;
pub(crate) mod metrics;
mod path;
mod pipeline;
mod processor;
pub use decoder::Decoder;
pub use device::Session;
pub use pipeline::NativePipeline;

fn native_error(message: &str, error: windows::core::Error) -> String {
    log::warn!("Native video: {message}: {error:?}");
    message.into()
}
#[cfg(test)]
mod tests;
