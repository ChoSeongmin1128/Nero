use serde_json::Value;
use std::sync::LazyLock;

static SPEC: LazyLock<Value> = LazyLock::new(|| {
    serde_json::from_str(include_str!(
        "../../../scripts/native-diarization/runtime.json"
    ))
    .expect("bundled native speech runtime specification")
});
static BASE: LazyLock<String> = LazyLock::new(|| {
    format!(
        "nemo-speech-{}@{}",
        SPEC["version"].as_str().unwrap().trim_start_matches('v'),
        SPEC["commit"].as_str().unwrap()
    )
});

pub fn current() -> &'static str {
    for_target(if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "macos-arm64"
    } else {
        "windows-x64"
    })
}

pub fn pin() -> &'static str {
    current().split_once('@').unwrap().1
}

pub fn known(runtime: &str) -> bool {
    runtime == BASE.as_str()
        || SPEC["sourceBuilds"].as_object().is_some_and(|profiles| {
            profiles
                .values()
                .any(|profile| profile["identity"].as_str() == Some(runtime))
        })
}

fn for_target(target: &str) -> &'static str {
    SPEC["sourceBuilds"][target]["identity"]
        .as_str()
        .unwrap_or(BASE.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_patch_identity_is_distinct_and_windows_keeps_its_official_runtime() {
        let mac = for_target("macos-arm64");
        assert_ne!(mac, BASE.as_str());
        assert_eq!(for_target("windows-x64"), BASE.as_str());
        let recipe = &SPEC["sourceBuilds"]["macos-arm64"];
        assert_eq!(
            mac.split_once('@').unwrap().1,
            recipe["pin"].as_str().unwrap()
        );
        assert!(mac.ends_with(recipe["patch"]["sha256"].as_str().unwrap()));
        assert!(known(&BASE));
        assert!(known(mac));
        assert!(!known("nemo-speech-0.2.0@unverified"));
        assert!(!known(&format!("{mac}-changed")));
    }
}
