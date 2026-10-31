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
    fn target_recipe_identities_are_distinct_and_base_artifacts_remain_readable() {
        let mac = for_target("macos-arm64");
        let windows = for_target("windows-x64");
        assert_ne!(mac, BASE.as_str());
        assert_ne!(windows, BASE.as_str());
        assert_ne!(windows, mac);
        assert_eq!(
            SPEC["sourceBuilds"]["windows-x64"]["sourceBackends"],
            serde_json::json!(["cpu"])
        );
        for target in ["macos-arm64", "windows-x64"] {
            let identity = for_target(target);
            let recipe = &SPEC["sourceBuilds"][target];
            assert_eq!(
                identity.split_once('@').unwrap().1,
                recipe["pin"].as_str().unwrap()
            );
            assert!(identity.contains(recipe["patch"]["sha256"].as_str().unwrap()));
            if let Some(hash) = recipe["windowsPatch"]["sha256"].as_str() {
                assert!(identity.ends_with(hash));
            }
            assert!(known(identity));
            assert!(!known(&format!("{identity}-changed")));
        }
        assert!(known(&BASE));
        assert!(!known("nemo-speech-0.2.0@unverified"));
    }
}
