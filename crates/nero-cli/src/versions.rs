use nero_core::agent_control::{self, AppMetadata};
use serde_json::{json, Value};
use std::path::Path;

pub(crate) fn integration_version() -> String {
    let release: Value =
        serde_json::from_str(include_str!("../../../apps/desktop/agent/release.json"))
            .expect("packaged integration release");
    release["integrationVersion"]
        .as_str()
        .expect("integrationVersion")
        .into()
}

pub(crate) fn status(root: &Path) -> Value {
    let app = agent_control::request(root, "app_metadata", json!({}));
    let app = app.and_then(|value| {
        serde_json::from_value::<AppMetadata>(value).map_err(|_| {
            agent_control::ControlError::new("invalid_response", "Invalid app metadata")
        })
    });
    comparison(app)
}

fn comparison(app: Result<AppMetadata, agent_control::ControlError>) -> Value {
    let running = integration_version();
    let (bundled, matches) = match app {
        Ok(app) => {
            let matches = app
                .integration_version
                .as_ref()
                .map(|version| version == &running);
            (
                json!({"state":"running","version":app.version,"build":app.build,"integrationVersion":app.integration_version}),
                matches,
            )
        }
        Err(error) => (json!({"state":"unavailable","reason":error.code}), None),
    };
    json!({
        "running":{"version":env!("CARGO_PKG_VERSION"),"integrationVersion":running,"apiVersion":1},
        "app":bundled,
        "matchesAppBundle":matches,
        "publicIntegration":{"state":"unknown","integrationVersion":null}
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stopped_app_status_does_not_initialize_a_library_or_claim_latest() {
        let parent = tempfile::tempdir().unwrap();
        let root = parent.path().join("missing");
        let versions = status(&root);
        assert!(!root.exists());
        assert_eq!(
            versions["running"]["integrationVersion"],
            integration_version()
        );
        assert_eq!(versions["app"]["state"], "unavailable");
        assert!(versions["matchesAppBundle"].is_null());
        assert_eq!(versions["publicIntegration"]["state"], "unknown");
    }

    #[test]
    fn compares_the_executing_binary_with_the_authenticated_app_bundle() {
        for (version, matches) in [
            (Some(integration_version()), Some(true)),
            (Some("0.0.1-nero.1".into()), Some(false)),
            (None, None),
        ] {
            let result = comparison(Ok(AppMetadata {
                version: "0.1.0".into(),
                build: "47".into(),
                integration_version: version.clone(),
            }));
            assert_eq!(result["app"]["state"], "running");
            assert_eq!(result["app"]["integrationVersion"], json!(version));
            assert_eq!(result["matchesAppBundle"], json!(matches));
            assert_eq!(result["publicIntegration"]["state"], "unknown");
        }
    }
}
