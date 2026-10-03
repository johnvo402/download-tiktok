//! Host process metrics, not GPU timestamp queries. FFmpeg child processes are excluded.
use std::time::Instant;
use windows::Win32::{
    Foundation::FILETIME,
    System::{
        ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS},
        Threading::{GetCurrentProcess, GetProcessTimes},
    },
};

fn cpu_seconds() -> Option<f64> {
    process_cpu_seconds(unsafe { GetCurrentProcess() })
}
pub(crate) fn process_cpu_seconds(handle: windows::Win32::Foundation::HANDLE) -> Option<f64> {
    let (mut created, mut ended, mut kernel, mut user) = (
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
    );
    unsafe {
        GetProcessTimes(handle, &mut created, &mut ended, &mut kernel, &mut user).ok()?;
    }
    let value = |t: FILETIME| ((t.dwHighDateTime as u64) << 32) | t.dwLowDateTime as u64;
    Some((value(kernel) + value(user)) as f64 / 10_000_000.0)
}
pub(crate) struct HostMetrics {
    start: Instant,
    cpu: Option<f64>,
    sampled_peak: usize,
    last_sample: Instant,
}
impl HostMetrics {
    pub(crate) fn total_cpu_percent(&self, child_seconds: f64) -> Option<f64> {
        self.cpu.zip(cpu_seconds()).map(|(before, after)| {
            let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
            (after - before + child_seconds)
                / self.start.elapsed().as_secs_f64().max(0.001)
                / cores as f64
                * 100.0
        })
    }
    pub(crate) fn new() -> Self {
        let mut result = Self {
            start: Instant::now(),
            cpu: cpu_seconds(),
            sampled_peak: 0,
            last_sample: Instant::now(),
        };
        result.sample();
        result
    }
    pub(crate) fn sample_throttled(&mut self) {
        if self.last_sample.elapsed().as_millis() >= 100 {
            self.sample();
        }
    }
    fn sample(&mut self) {
        let mut memory = PROCESS_MEMORY_COUNTERS::default();
        let size = std::mem::size_of_val(&memory) as u32;
        memory.cb = size;
        if unsafe { GetProcessMemoryInfo(GetCurrentProcess(), &mut memory, size) }.is_ok() {
            self.sampled_peak = self.sampled_peak.max(memory.WorkingSetSize);
        }
        self.last_sample = Instant::now();
    }
    pub(crate) fn report(&mut self, label: &str) -> String {
        self.sample();
        let cpu = self.total_cpu_percent(0.0);
        format!("{label}: Rust process CPU={cpu:?}%; sampled peak working set={:.1}MiB (children excluded)", self.sampled_peak as f64 / 1048576.0)
    }
}
