//! Lazy, isolated workers. A failed job cannot leave a Python/browser tree alive.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::ops::{Deref, DerefMut};
use std::process::{Child, Command, Output, Stdio};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

pub const PROTOCOL_VERSION: u64 = 2;
pub struct BoundedLines<R> { reader:R, maximum:u64, done:bool }
pub fn bounded_lines<R:BufRead>(reader:R, maximum:u64)->BoundedLines<R> {BoundedLines {reader,maximum,done:false}}
impl<R:BufRead> Iterator for BoundedLines<R> {
    type Item=io::Result<String>;
    fn next(&mut self)->Option<Self::Item> {
        if self.done {return None;}
        let mut bytes=Vec::new();
        match self.reader.by_ref().take(self.maximum+1).read_until(b'\n',&mut bytes) {
            Ok(0)=>{self.done=true;None},
            Ok(count) if count as u64>self.maximum=>{self.done=true;Some(Err(io::Error::new(io::ErrorKind::InvalidData,"Process reply exceeds the allowed size")))},
            Ok(_)=>{if bytes.last()==Some(&b'\n') {bytes.pop();} Some(Ok(String::from_utf8_lossy(&bytes).into_owned()))},
            Err(error)=>{self.done=true;Some(Err(error))},
        }
    }
}
pub fn read_tail(mut pipe:impl Read, maximum:usize)->io::Result<String> {
    let mut bytes=Vec::new();let mut chunk=[0_u8;8192];
    loop {let count=pipe.read(&mut chunk)?;if count==0 {break;}bytes.extend_from_slice(&chunk[..count]);if bytes.len()>maximum {bytes.drain(..bytes.len()-maximum);}}
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}
struct ActiveProcess { executable: PathBuf, scope: Arc<ProcessScope> }
static ACTIVE: OnceLock<Mutex<HashMap<u32, ActiveProcess>>> = OnceLock::new();
fn active() -> &'static Mutex<HashMap<u32, ActiveProcess>> { ACTIVE.get_or_init(Mutex::default) }
struct ProcessScope {
    #[cfg(windows)] job: Option<windows::Win32::Foundation::HANDLE>,
    #[cfg(windows)] process: windows::Win32::Foundation::HANDLE,
}
// Owned kernel handles may be closed/signalled from another thread. No borrowed
// handle or Python state is shared between workers.
unsafe impl Send for ProcessScope {}
unsafe impl Sync for ProcessScope {}
impl ProcessScope {
    fn stop(&self) {
        #[cfg(windows)] unsafe {
            if let Some(job) = self.job {
                let _ = windows::Win32::System::JobObjects::TerminateJobObject(job, 1);
            } else {
                let _ = windows::Win32::System::Threading::TerminateProcess(self.process, 1);
            }
        }
    }
}
impl Drop for ProcessScope {
    fn drop(&mut self) {
        #[cfg(windows)] unsafe {
            if let Some(job) = self.job { let _ = windows::Win32::Foundation::CloseHandle(job); }
            let _ = windows::Win32::Foundation::CloseHandle(self.process);
        }
    }
}
pub fn with_runtime_idle<T>(directory: &Path, action: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    let registry = active().lock().map_err(|_| "Không thể kiểm tra tác vụ đang chạy.")?;
    if registry.values().any(|worker| worker.executable.starts_with(directory)) {
        return Err("Bộ xử lý đang được sử dụng. Hãy chờ tác vụ hoàn tất rồi thử lại.".into());
    }
    // Spawn also takes this lock, preventing a new worker racing with promotion.
    action()
}

pub struct ManagedWorker {
    child: Option<Child>,
    started: Instant,
    scope: Arc<ProcessScope>,
}
impl Deref for ManagedWorker {
    type Target = Child;
    fn deref(&self) -> &Child { self.child.as_ref().expect("live worker handle") }
}
impl DerefMut for ManagedWorker {
    fn deref_mut(&mut self) -> &mut Child { self.child.as_mut().expect("live worker handle") }
}
impl ManagedWorker {
    /// Kill owned descendants before waiting for pipe readers to reach EOF.
    pub fn kill(&mut self) -> io::Result<()> {
        self.scope.stop();
        self.child.as_mut().expect("live worker handle").kill()
    }
    pub fn wait_with_output(self) -> io::Result<Output> { self.wait_with_output_timeout(Duration::from_secs(300)) }
    pub fn wait_with_output_timeout(mut self, timeout: Duration) -> io::Result<Output> {
        drop(self.stdin.take());
        let stdout = self.stdout.take();
        let stderr = self.stderr.take();
        let read = |pipe: Option<Box<dyn Read + Send>>| std::thread::spawn(move || {
            let mut bytes = Vec::new();
            if let Some(mut pipe) = pipe {
                let mut chunk = [0_u8; 8192];
                loop {
                    let count = pipe.read(&mut chunk)?;
                    if count == 0 { break; }
                    bytes.extend_from_slice(&chunk[..count]);
                    if bytes.len() > 8 * 1024 * 1024 { bytes.drain(..bytes.len() - 8 * 1024 * 1024); }
                }
            }
            Ok::<_, io::Error>(bytes)
        });
        let out = read(stdout.map(|p| Box::new(p) as Box<dyn Read + Send>));
        let err = read(stderr.map(|p| Box::new(p) as Box<dyn Read + Send>));
        let status = loop {
            if let Some(status) = self.try_wait()? { break status; }
            if self.started.elapsed() > timeout {
                terminate_tree(&mut self);
                return Err(io::Error::new(io::ErrorKind::TimedOut, "Worker inspection timed out."));
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        // A descendant could inherit pipes after its parent exits. Stop the
        // contained tree before joining readers, so EOF cannot wait forever.
        self.scope.stop();
        let stdout = out.join().map_err(|_| io::Error::other("Worker stdout reader failed"))??;
        let stderr = err.join().map_err(|_| io::Error::other("Worker stderr reader failed"))??;
        Ok(Output {status, stdout, stderr})
    }
}
impl Drop for ManagedWorker {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            if let Ok(mut registry) = active().lock() { registry.remove(&child.id()); }
            self.scope.stop();
            if !matches!(child.try_wait(), Ok(Some(_))) { terminate_tree(&mut child); }
        }
    }
}

pub trait WorkerCommandExt {
    fn spawn_worker(&mut self) -> io::Result<ManagedWorker>;
    fn spawn_contained(&mut self) -> io::Result<ManagedWorker>;
    fn output_contained(&mut self, timeout: Duration) -> io::Result<Output>;
}
impl WorkerCommandExt for Command {
    fn output_contained(&mut self, timeout: Duration) -> io::Result<Output> { output_timeout(self, timeout) }
    fn spawn_contained(&mut self) -> io::Result<ManagedWorker> { spawn_process(self) }
    fn spawn_worker(&mut self) -> io::Result<ManagedWorker> {
        // Ignore arbitrary global Python homes/startup scripts. Each command explicitly
        // supplies its own source path or a frozen executable and selected environment.
        self.env_remove("PYTHONHOME").env_remove("PYTHONSTARTUP").env_remove("PYTHONUSERBASE")
            .env("PYTHONNOUSERSITE", "1").env("PYTHONDONTWRITEBYTECODE", "1")
            .env("JC_SESSION_ID", crate::app_session::id())
            .env("PYTHONUNBUFFERED", "1").env("PYTHONUTF8", "1")
            .stdin(Stdio::piped()).stdout(Stdio::piped());
        let mut managed = spawn_process(self)?;
        // A standard-library-only handshake is cheap and has a bounded deadline.
        managed.stdin.as_mut().ok_or_else(|| io::Error::other("Worker stdin unavailable"))?
            .write_all(b"{\"operation\":\"ping\",\"protocolVersion\":2}\n")?;
        let stdout = managed.stdout.take().ok_or_else(|| io::Error::other("Worker stdout unavailable"))?;
        let (tx, rx) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            let mut line = String::new();
            let result = reader.by_ref().take(65537).read_line(&mut line).and_then(|_| {
                if line.len() > 65536 { return Err(io::Error::other("Worker startup reply is too large")); }
                validate_handshake(&line)
            });
            let _ = tx.send((result, reader.into_inner()));
        });
        let (result, stdout) = rx.recv_timeout(Duration::from_secs(30))
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "Worker did not become ready in 30 seconds."))?;
        result?;
        managed.stdout = Some(stdout);
        Ok(managed)
    }
}
pub fn spawn_process(command: &mut Command) -> io::Result<ManagedWorker> {
    #[cfg(windows)] {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut registry = active().lock().map_err(|_|io::Error::other("Process registry unavailable"))?;
    let mut child = command.spawn()?;
    #[cfg(windows)]
    let scope = {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::{Foundation::{DuplicateHandle, HANDLE, DUPLICATE_SAME_ACCESS}, System::Threading::GetCurrentProcess};
        let mut process = HANDLE::default();
        if let Err(error) = unsafe { DuplicateHandle(GetCurrentProcess(), HANDLE(child.as_raw_handle() as _), GetCurrentProcess(), &mut process, 0, false, DUPLICATE_SAME_ACCESS) } {
            terminate_tree(&mut child);
            return Err(io::Error::other(error.to_string()));
        }
        Arc::new(ProcessScope { job: containment(&child), process })
    };
    #[cfg(not(windows))] let scope = Arc::new(ProcessScope {});
    let managed = ManagedWorker {child:Some(child),started:Instant::now(),scope:scope.clone()};
    registry.insert(managed.id(), ActiveProcess {executable:PathBuf::from(command.get_program()),scope});
    Ok(managed)
}
pub fn output_timeout(command: &mut Command, timeout: Duration) -> io::Result<Output> {
    // Match Command::output semantics; spawn otherwise inherits the console.
    command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    spawn_process(command)?.wait_with_output_timeout(timeout)
}
fn validate_handshake(line: &str) -> io::Result<()> {
    let message: serde_json::Value = serde_json::from_str(line)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "Worker returned invalid startup JSON."))?;
    if message["type"] != "ready" || message["protocolVersion"].as_u64() != Some(PROTOCOL_VERSION) {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "Worker protocol/version is incompatible. Reinstall JC AI."));
    }
    Ok(())
}

#[cfg(windows)]
fn containment(child: &Child) -> Option<windows::Win32::Foundation::HANDLE> {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::{Foundation::{CloseHandle, HANDLE}, System::JobObjects::*};
    unsafe {
        let job = CreateJobObjectW(None, None).ok()?;
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if SetInformationJobObject(job, JobObjectExtendedLimitInformation, &limits as *const _ as _,
                std::mem::size_of_val(&limits) as u32).is_err()
            || AssignProcessToJobObject(job, HANDLE(child.as_raw_handle() as _)).is_err() {
            let _ = CloseHandle(job);
            return None;
        }
        Some(job)
    }
}
pub fn terminate_tree(child: &mut Child) {
    #[cfg(windows)] {
        use std::os::windows::process::CommandExt;
        let _ = Command::new("taskkill.exe").args(["/PID", &child.id().to_string(), "/T", "/F"])
            .creation_flags(0x08000000).stdout(Stdio::null()).stderr(Stdio::null()).status();
    }
    let _ = child.kill();
    let _ = child.wait();
}
pub fn shutdown() {
    let scopes = active().lock().map(|r| r.values().map(|p|p.scope.clone()).collect::<Vec<_>>()).unwrap_or_default();
    for scope in scopes { scope.stop(); }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn protocol_records_and_logs_have_explicit_memory_bounds() {
        let mut lines=bounded_lines(io::Cursor::new(b"small\nxxxxxxxxxxxxxxxxxxxxxxxx"),8);
        assert_eq!(lines.next().unwrap().unwrap(),"small");assert!(lines.next().unwrap().is_err());assert!(lines.next().is_none());
        assert_eq!(read_tail(io::Cursor::new(b"0123456789"),4).unwrap(),"6789");
    }
    #[test] fn startup_protocol_is_strict() {
        assert!(validate_handshake(r#"{"type":"ready","protocolVersion":2}"#).is_ok());
        for invalid in ["Traceback", "{}", r#"{"type":"ready","protocolVersion":1}"#, r#"{"type":"failed","protocolVersion":2}"#] {
            assert!(validate_handshake(invalid).is_err());
        }
    }
    #[cfg(windows)]
    fn python(code: &str) -> Command {
        use std::os::windows::process::CommandExt;
        let mut command = Command::new("python");
        command.args(["-c",code]).creation_flags(0x08000000)
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        command
    }
    #[cfg(windows)]
    #[test] fn inspection_deadline_kills_process() {
        let mut command=python("import time; time.sleep(60)");
        let began=Instant::now();
        let error=output_timeout(&mut command,Duration::from_millis(100)).unwrap_err();
        assert_eq!(error.kind(),io::ErrorKind::TimedOut);
        assert!(began.elapsed()<Duration::from_secs(5));
    }
    #[cfg(windows)]
    #[test] fn inherited_pipes_do_not_wait_for_orphan() {
        let mut command=python("import subprocess,sys; subprocess.Popen([sys.executable,'-c','import time; time.sleep(60)']); print('parent done',flush=True)");
        let began=Instant::now();
        let output=output_timeout(&mut command,Duration::from_secs(5)).unwrap();
        assert!(output.status.success());
        assert!(String::from_utf8_lossy(&output.stdout).contains("parent done"));
        assert!(began.elapsed()<Duration::from_secs(5));
    }
    #[cfg(windows)]
    #[test] fn output_is_bounded() {
        let mut command=python("import sys; sys.stdout.write('x'*(9*1024*1024)); sys.stderr.write('y'*(9*1024*1024))");
        let output=output_timeout(&mut command,Duration::from_secs(10)).unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout.len(),8*1024*1024);
        assert_eq!(output.stderr.len(),8*1024*1024);
    }
    #[cfg(windows)]
    #[test] fn old_protocol_worker_is_rejected() {
        let mut command=python("import json; input(); print(json.dumps({'type':'ready','protocolVersion':1}),flush=True)");
        assert!(command.spawn_worker().is_err());
    }
}
