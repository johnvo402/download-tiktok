use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=GIT_COMMIT");
    let commit = std::env::var("GIT_COMMIT")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            Command::new("git")
                .args(["rev-parse", "--short=12", "HEAD"])
                .output()
                .ok()
                .filter(|output| output.status.success())
                .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
                .filter(|value| !value.is_empty())
        })
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=GIT_COMMIT={commit}");
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");
    let epoch = std::env::var("SOURCE_DATE_EPOCH").ok().and_then(|v|v.parse::<u64>().ok())
        .unwrap_or_else(||std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs());
    println!("cargo:rustc-env=JC_BUILD_EPOCH={epoch}");
    let branch=Command::new("git").args(["symbolic-ref","-q","HEAD"]).output().ok().filter(|o|o.status.success()).map(|o|String::from_utf8_lossy(&o.stdout).trim().to_string());
    for reference in std::iter::once("HEAD").chain(branch.as_deref()) {
        if let Ok(output)=Command::new("git").args(["rev-parse","--git-path",reference]).output() {
            if output.status.success() {println!("cargo:rerun-if-changed={}",String::from_utf8_lossy(&output.stdout).trim());}
        }
    }
    tauri_build::build()
}
