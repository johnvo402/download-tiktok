//! Hardware frame transport. Native pointers remain inside the Windows boundary.
pub mod capability;
pub mod frame;
pub mod pipeline;
#[cfg(windows)]
pub mod windows;
