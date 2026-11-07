// How Claude Code's model rows are shown and which effort a request starts with.

// Claude Code model configuration, reviewed 2026-11-05:
// https://code.claude.com/docs/en/model-config
// "high on every model that supports effort, except that Opus 5.5, Sonnet 5.5,
// and Haiku 5.5 default to medium and Opus 4.7 defaults to xhigh."
fn default_effort(model: &str, levels: &[String]) -> Option<String> {
    let model = model.split('[').next().unwrap_or(model);
    let named = |id: &str| model == id || model.starts_with(&format!("{id}-"));
    let level = if ["claude-opus-5-5", "claude-sonnet-5-5", "claude-haiku-5-5"]
        .into_iter()
        .any(named)
    {
        "medium"
    } else if named("claude-opus-4-7") {
        "xhigh"
    } else {
        "high"
    };
    levels
        .iter()
        .any(|value| value == level)
        .then(|| level.to_owned())
}
fn release(model: &str) -> Option<(String, Vec<u32>)> {
    let model = model.split('[').next()?;
    let mut parts = model.strip_prefix("claude-")?.split('-');
    let family = parts
        .next()
        .filter(|family| !family.is_empty() && family.bytes().all(|c| c.is_ascii_lowercase()))?;
    let mut version = Vec::new();
    for part in parts {
        // A trailing eight-digit part is the release date, not a version number.
        if part.len() == 8 && part.bytes().all(|c| c.is_ascii_digit()) {
            break;
        }
        version.push(part.parse().ok()?);
    }
    (!version.is_empty()).then(|| (family.to_owned(), version))
}
// The CLI's "default" row is an alias of another listed model. Rows whose family has a
// newer listed release are earlier models. A bare family name gets the version of the
// model the CLI resolved it to.
pub(super) fn chat_models(
    models: Vec<crate::claude_runner::SupportedModel>,
) -> Vec<super::provider::ChatModel> {
    let models = models
        .into_iter()
        .filter(|model| model.value != "default")
        .collect::<Vec<_>>();
    let releases = models
        .iter()
        .map(|model| release(model.resolved_model.as_deref().unwrap_or(&model.value)))
        .collect::<Vec<_>>();
    models
        .into_iter()
        .zip(releases.iter())
        .map(|(model, release)| {
            let resolved = model.resolved_model.as_deref().unwrap_or(&model.value);
            let default = (model.supports_effort == Some(true))
                .then(|| default_effort(resolved, &model.supported_effort_levels))
                .flatten();
            let previous = release.as_ref().is_some_and(|(family, version)| {
                releases
                    .iter()
                    .flatten()
                    .any(|(other, newer)| other == family && newer > version)
            });
            let label = match release {
                Some((family, version))
                    if !model.display_name.bytes().any(|c| c.is_ascii_digit())
                        && model.display_name.eq_ignore_ascii_case(family) =>
                {
                    let version = version.iter().map(u32::to_string).collect::<Vec<_>>();
                    format!("{} {}", model.display_name, version.join("."))
                }
                _ => model.display_name,
            };
            super::provider::ChatModel {
                previous,
                effort: super::provider::EffortCapability {
                    supported: model.supports_effort,
                    levels: model.supported_effort_levels,
                    default,
                },
                id: model.value,
                label,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn claude_models_show_names_hide_the_default_alias_and_apply_documented_efforts() {
        let row = |value: &str, name: &str, resolved: &str, levels: &[&str]| {
            crate::claude_runner::SupportedModel {
                value: value.into(),
                display_name: name.into(),
                resolved_model: Some(resolved.into()),
                supports_effort: (!levels.is_empty()).then_some(true),
                supported_effort_levels: levels.iter().map(|v| (*v).to_string()).collect(),
            }
        };
        let all = ["low", "medium", "high", "xhigh", "max"];
        let models = chat_models(vec![
            row("default", "Default (recommended)", "claude-opus-5-5", &all),
            row("opus", "Opus 5.5", "claude-opus-5-5", &all),
            row("sonnet", "Sonnet 5.5", "claude-sonnet-5-5", &all),
            row("fable", "Fable 5.1", "claude-fable-5-1", &all),
            row("claude-opus-4-7", "Opus 4.7", "claude-opus-4-7", &all),
            row(
                "claude-sonnet-4-6",
                "Sonnet 4.6",
                "claude-sonnet-4-6",
                &["low", "medium", "high", "max"],
            ),
            row(
                "claude-haiku-4-5-20251001",
                "Haiku 4.5",
                "claude-haiku-4-5-20251001",
                &[],
            ),
        ]);
        let view = models
            .iter()
            .map(|m| {
                (
                    m.id.as_str(),
                    m.label.as_str(),
                    m.previous,
                    m.effort.default.as_deref(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            view,
            [
                ("opus", "Opus 5.5", false, Some("medium")),
                ("sonnet", "Sonnet 5.5", false, Some("medium")),
                ("fable", "Fable 5.1", false, Some("high")),
                ("claude-opus-4-7", "Opus 4.7", true, Some("xhigh")),
                ("claude-sonnet-4-6", "Sonnet 4.6", true, Some("high")),
                ("claude-haiku-4-5-20251001", "Haiku 4.5", false, None),
            ]
        );
        let bare = chat_models(vec![
            row("default", "Default (recommended)", "claude-opus-5-5", &all),
            row("opus", "Opus", "claude-opus-5-5", &all),
            row("claude-fable-5-1[1m]", "Fable", "claude-fable-5-1", &all),
            row("sonnet", "Sonnet", "claude-sonnet-5-5", &all),
            row("haiku", "Haiku", "claude-haiku-5-5", &all),
        ]);
        assert_eq!(
            bare.iter()
                .map(|m| (m.label.as_str(), m.previous))
                .collect::<Vec<_>>(),
            [
                ("Opus 5.5", false),
                ("Fable 5.1", false),
                ("Sonnet 5.5", false),
                ("Haiku 5.5", false)
            ]
        );
        assert_eq!(
            release("claude-haiku-4-5-20251001"),
            Some(("haiku".to_string(), vec![4, 5]))
        );
        assert_eq!(release("claude-3-5-sonnet-20241022"), None);
        assert_eq!(
            default_effort("claude-opus-5-5[1m]", &["medium".into()]).as_deref(),
            Some("medium")
        );
        assert_eq!(
            default_effort("claude-opus-5", &["medium".into(), "high".into()]).as_deref(),
            Some("high")
        );
        assert_eq!(default_effort("claude-opus-4-7", &["high".into()]), None);
    }
}
