//! Single versioned dependency graph shared with packaging. No model weights.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Registry {
    pub runtime_version: u32,
    pub protocol_version: u64,
    pub python_version: String,
    pub features: BTreeMap<String, Feature>,
    pub components: BTreeMap<String, Component>,
    pub pins: BTreeMap<String, String>,
}
#[derive(Deserialize)]
pub struct Feature { pub backends: Vec<String>, pub components: Vec<String>, pub requirements: String }
#[derive(Deserialize, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Component {
    pub label: String, pub download_bytes: u64, pub installed_bytes: u64,
    pub packages: Vec<String>, pub depends_on: Vec<String>,
}
pub fn registry() -> &'static Registry {
    static VALUE: OnceLock<Registry> = OnceLock::new();
    VALUE.get_or_init(|| serde_json::from_str(include_str!("../../python/runtime-matrix.json")).expect("validated runtime matrix"))
}
pub fn components(features: &[String], backend: &str) -> Result<BTreeSet<String>, String> {
    let configuration = registry();
    let mut result = BTreeSet::new();
    fn add(id: &str, backend: &str, result: &mut BTreeSet<String>) -> Result<(), String> {
        let id = if id == "torch" { format!("torch_{backend}") } else { id.to_string() };
        if result.contains(&id) { return Ok(()); }
        let component = registry().components.get(&id).ok_or("Thành phần xử lý không hợp lệ.")?;
        result.insert(id);
        for dependency in &component.depends_on { add(dependency, backend, result)?; }
        Ok(())
    }
    for id in features {
        let feature = configuration.features.get(id).ok_or("Tính năng không hợp lệ.")?;
        if !feature.backends.iter().any(|b| b == backend) { return Err("Thiết bị không phù hợp với tính năng này.".into()); }
        for component in &feature.components { add(component, backend, &mut result)?; }
    }
    Ok(result)
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallPlan {
    pub feature: String, pub backend: String, pub runtime_version: u32,
    pub missing_components: Vec<Component>, pub reused_components: Vec<String>,
    pub estimated_download_bytes: u64, pub estimated_installed_bytes: u64,
    pub staging_bytes: u64, pub already_installed: bool,
}
pub fn plan(feature: &str, backend: &str, installed: &[String], staging_bytes: u64) -> Result<InstallPlan, String> {
    let requested = components(&[feature.into()], backend)?;
    let existing = components(installed, backend)?;
    let missing = requested.difference(&existing).map(|id| registry().components[id].clone()).collect::<Vec<_>>();
    Ok(InstallPlan {
        feature: feature.into(), backend: backend.into(), runtime_version: registry().runtime_version,
        estimated_download_bytes: missing.iter().map(|c| c.download_bytes).sum(),
        estimated_installed_bytes: missing.iter().map(|c| c.installed_bytes).sum(),
        reused_components: requested.intersection(&existing).map(|id| registry().components[id].label.clone()).collect(),
        missing_components: missing, staging_bytes, already_installed: installed.iter().any(|id| id == feature),
    })
}
pub fn removable_packages(feature: &str, remaining: &[String], backend: &str) -> Result<Vec<String>, String> {
    let old = components(&[feature.into()], backend)?;
    let keep = components(remaining, backend)?;
    let keep_packages = keep.iter().flat_map(|id| registry().components[id].packages.iter()).collect::<BTreeSet<_>>();
    // Package lists declare ownership of direct dependencies. Transitive packages
    // are conservatively retained; shared dependencies never get removed blindly.
    Ok(old.difference(&keep).flat_map(|id| registry().components[id].packages.iter())
        .filter(|p| !keep_packages.contains(p)).cloned().collect::<BTreeSet<_>>().into_iter().collect())
}
#[cfg(test)] mod tests {
    use super::*;
    #[test] fn shared_torch_downloads_once_and_intel_is_independent() {
        let first = plan("sttn", "cuda", &[], 0).unwrap();
        let second = plan("bandit", "cuda", &["sttn".into()], 400).unwrap();
        assert!(first.estimated_download_bytes > second.estimated_download_bytes);
        assert_eq!(second.reused_components.len(), 1);
        assert_eq!(second.missing_components.len(), 1);
        assert!(components(&["ocr_openvino".into()], "openvino").unwrap().iter().all(|id| !id.starts_with("torch")));
    }
    #[test] fn uninstall_preserves_shared_packages() {
        let packages = removable_packages("whisperx", &["sttn".into(), "acceleration".into()], "cuda").unwrap();
        for protected in ["torch", "torchaudio", "transformers", "huggingface-hub"] { assert!(!packages.iter().any(|p| p == protected)); }
        assert!(packages.iter().any(|p| p == "whisperx"));
    }
    #[test] fn invalid_profile_and_feature_fail() {
        assert!(plan("unknown", "cpu", &[], 0).is_err());
        assert!(plan("sttn", "openvino", &[], 0).is_err());
        assert_eq!(registry().protocol_version, crate::worker_manager::PROTOCOL_VERSION);
    }
}
