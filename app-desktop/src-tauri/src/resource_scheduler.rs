use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock, TryLockError};
use std::thread;
use std::time::Duration;

static AI_GPU_SLOT: OnceLock<Mutex<()>> = OnceLock::new();
static CPU_SLOT: OnceLock<Mutex<()>> = OnceLock::new();
static FFMPEG_SLOT: OnceLock<Mutex<()>> = OnceLock::new();

pub fn ai_gpu_slot() -> &'static Mutex<()> {
    AI_GPU_SLOT.get_or_init(|| Mutex::new(()))
}

pub fn ffmpeg_slot() -> &'static Mutex<()> {
    FFMPEG_SLOT.get_or_init(|| Mutex::new(()))
}

pub fn cpu_slot() -> &'static Mutex<()> {
    CPU_SLOT.get_or_init(|| Mutex::new(()))
}

pub fn wait_for_ai_gpu(canceled: &AtomicBool) -> Result<MutexGuard<'static, ()>, String> {
    loop {
        if canceled.load(Ordering::SeqCst) {
            return Err("Đã hủy khi đang chờ tài nguyên GPU.".into());
        }
        match ai_gpu_slot().try_lock() {
            Ok(guard) => return Ok(guard),
            Err(TryLockError::Poisoned(poisoned)) => return Ok(poisoned.into_inner()),
            Err(TryLockError::WouldBlock) => thread::sleep(Duration::from_millis(100)),
        }
    }
}

pub fn wait_for_cpu(canceled: &AtomicBool) -> Result<MutexGuard<'static, ()>, String> {
    loop {
        if canceled.load(Ordering::SeqCst) {
            return Err("Đã hủy khi đang chờ tài nguyên CPU.".into());
        }
        match cpu_slot().try_lock() {
            Ok(guard) => return Ok(guard),
            Err(TryLockError::Poisoned(poisoned)) => return Ok(poisoned.into_inner()),
            Err(TryLockError::WouldBlock) => thread::sleep(Duration::from_millis(100)),
        }
    }
}

pub fn try_cpu() -> Result<MutexGuard<'static, ()>, TryLockError<MutexGuard<'static, ()>>> {
    cpu_slot().try_lock()
}

pub fn try_ffmpeg() -> Result<MutexGuard<'static, ()>, TryLockError<MutexGuard<'static, ()>>> {
    ffmpeg_slot().try_lock()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gpu_slot_serializes_heavy_model_jobs() {
        let _first = ai_gpu_slot().lock().unwrap();
        assert!(matches!(
            ai_gpu_slot().try_lock(),
            Err(TryLockError::WouldBlock)
        ));
    }

    #[test]
    fn ffmpeg_slot_serializes_exports() {
        let _first = ffmpeg_slot().lock().unwrap();
        assert!(matches!(try_ffmpeg(), Err(TryLockError::WouldBlock)));
    }

    #[test]
    fn cpu_slot_serializes_heavy_cpu_jobs() {
        let _first = cpu_slot().lock().unwrap();
        assert!(matches!(try_cpu(), Err(TryLockError::WouldBlock)));
    }
}
