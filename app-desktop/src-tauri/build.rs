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
    tauri_build::build()
}
