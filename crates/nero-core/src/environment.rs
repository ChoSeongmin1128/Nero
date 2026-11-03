#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Profile {
    pub identifier: &'static str,
    pub app_name: &'static str,
    pub connections_directory: &'static str,
    pub ai_credentials_service: &'static str,
    pub notion_credentials_service: &'static str,
    pub calendar_credentials_service: &'static str,
    pub legacy_calendar_service: Option<&'static str>,
    pub development: bool,
}

pub const PRODUCTION: Profile = Profile {
    identifier: "io.github.choseongmin1128.nero.desktop",
    app_name: "Nero",
    connections_directory: "Nero",
    ai_credentials_service: "io.github.choseongmin1128.nero.desktop.ai-accounts",
    notion_credentials_service: "io.github.choseongmin1128.nero.desktop.notion-context",
    calendar_credentials_service: "io.github.choseongmin1128.nero.desktop.google-calendar",
    legacy_calendar_service: Some("io.github.choseongmin1128.grove.desktop.google-calendar"),
    development: false,
};

pub const DEVELOPMENT: Profile = Profile {
    identifier: "io.github.choseongmin1128.nero.desktop.dev",
    app_name: "Nero Dev",
    connections_directory: "Nero Dev",
    ai_credentials_service: "io.github.choseongmin1128.nero.desktop.dev.ai-accounts",
    notion_credentials_service: "io.github.choseongmin1128.nero.desktop.dev.notion-context",
    calendar_credentials_service: "io.github.choseongmin1128.nero.desktop.dev.google-calendar",
    legacy_calendar_service: None,
    development: true,
};

pub const CURRENT: Profile = if cfg!(any(debug_assertions, feature = "development")) {
    DEVELOPMENT
} else {
    PRODUCTION
};

impl Profile {
    pub fn validate_bundle(self, identifier: &str, name: Option<&str>) -> Result<(), &'static str> {
        if identifier == self.identifier && name == Some(self.app_name) {
            Ok(())
        } else {
            Err("앱 빌드와 실행 환경이 일치하지 않습니다. 개발 빌드는 Nero Dev 설정으로 실행해 주세요.")
        }
    }
}

pub fn require_updates() -> Result<(), &'static str> {
    if CURRENT.development {
        Err("개발 앱에서는 자동 업데이트를 사용할 수 없습니다.")
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_identity_is_stable_and_development_has_separate_storage() {
        assert_eq!(PRODUCTION.identifier, crate::identity::APP_IDENTIFIER);
        assert_eq!(PRODUCTION.app_name, "Nero");
        assert_ne!(PRODUCTION.identifier, DEVELOPMENT.identifier);
        assert_ne!(
            PRODUCTION.connections_directory,
            DEVELOPMENT.connections_directory
        );
        assert_ne!(
            PRODUCTION.ai_credentials_service,
            DEVELOPMENT.ai_credentials_service
        );
        assert_ne!(
            PRODUCTION.notion_credentials_service,
            DEVELOPMENT.notion_credentials_service
        );
        assert_ne!(
            PRODUCTION.calendar_credentials_service,
            DEVELOPMENT.calendar_credentials_service
        );
        assert!(DEVELOPMENT.legacy_calendar_service.is_none());
    }

    #[test]
    fn compile_profile_controls_updates_without_a_runtime_override() {
        assert_eq!(
            CURRENT.development,
            cfg!(any(debug_assertions, feature = "development"))
        );
        assert_eq!(require_updates().is_err(), CURRENT.development);
    }

    #[test]
    fn bundle_configuration_cannot_select_another_profiles_storage() {
        for profile in [PRODUCTION, DEVELOPMENT] {
            assert!(profile
                .validate_bundle(profile.identifier, Some(profile.app_name))
                .is_ok());
            let other = if profile.development {
                PRODUCTION
            } else {
                DEVELOPMENT
            };
            assert!(profile
                .validate_bundle(other.identifier, Some(other.app_name))
                .is_err());
            assert!(profile.validate_bundle(profile.identifier, None).is_err());
        }
    }
}
