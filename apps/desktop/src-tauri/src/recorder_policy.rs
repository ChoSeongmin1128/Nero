#[derive(Debug, PartialEq, Eq)]
pub enum Presentation {
    Show,
    Hide,
}

pub struct RecorderPolicy {
    automatic: bool,
    session: Option<String>,
    background: bool,
    dismissed: bool,
    shown: bool,
    failed: bool,
    last_moved: Option<std::time::Instant>,
}

impl Default for RecorderPolicy {
    fn default() -> Self {
        Self {
            automatic: true,
            session: None,
            background: false,
            dismissed: false,
            shown: false,
            failed: false,
            last_moved: None,
        }
    }
}

impl RecorderPolicy {
    pub fn moved(&mut self, now: std::time::Instant) {
        self.last_moved = Some(now);
    }
    pub fn can_reposition(&self, now: std::time::Instant) -> bool {
        self.last_moved.is_none_or(|last| {
            now.saturating_duration_since(last) >= std::time::Duration::from_secs(1)
        })
    }
    pub fn set_automatic(&mut self, enabled: bool) {
        self.automatic = enabled;
        if enabled {
            self.dismissed = false;
            self.failed = false;
        }
    }

    pub fn dismiss(&mut self) {
        self.dismissed = true;
        self.shown = false;
    }

    pub fn show_failed(&mut self) {
        self.shown = false;
        self.failed = true;
    }

    pub fn update(&mut self, session: Option<&str>, background: bool) -> Option<Presentation> {
        if self.session.as_deref() != session {
            self.dismissed = false;
            self.failed = false;
        } else if self.background != background {
            self.failed = false;
        }
        self.session = session.map(str::to_owned);
        self.background = background;
        let visible = self.automatic && session.is_some() && !self.dismissed && !self.failed;
        if visible == self.shown {
            return None;
        }
        self.shown = visible;
        Some(if visible {
            Presentation::Show
        } else {
            Presentation::Hide
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_recording_shows_immediately_and_stays_visible_until_it_ends() {
        let mut policy = RecorderPolicy::default();
        assert_eq!(policy.update(None, true), None);
        assert_eq!(policy.update(Some("a"), false), Some(Presentation::Show));
        assert_eq!(policy.update(Some("a"), true), None);
        assert_eq!(policy.update(Some("a"), true), None);
        assert_eq!(policy.update(Some("a"), false), None);
        assert_eq!(policy.update(Some("a"), true), None);
        assert_eq!(policy.update(Some("a"), false), None);
        assert_eq!(policy.update(None, true), Some(Presentation::Hide));
        assert_eq!(policy.update(Some("b"), false), Some(Presentation::Show));
    }

    #[test]
    fn user_dismissal_survives_main_window_and_focus_transitions() {
        let mut policy = RecorderPolicy::default();
        policy.update(Some("a"), true);
        policy.dismiss();
        assert_eq!(policy.update(Some("a"), false), None);
        assert_eq!(policy.update(Some("a"), true), None);
        assert_eq!(policy.update(Some("b"), true), Some(Presentation::Show));
        policy.dismiss();
        assert_eq!(policy.update(Some("b"), false), None);
        assert_eq!(policy.update(Some("b"), true), None);
    }

    #[test]
    fn automatic_display_can_be_disabled_and_reenabled() {
        let mut policy = RecorderPolicy::default();
        policy.update(Some("a"), true);
        policy.set_automatic(false);
        assert_eq!(policy.update(Some("a"), true), Some(Presentation::Hide));
        assert_eq!(policy.update(Some("b"), true), None);
        policy.set_automatic(true);
        assert_eq!(policy.update(Some("b"), true), Some(Presentation::Show));
    }

    #[test]
    fn disabling_hides_a_presented_window_and_reenabling_resets_user_dismissal() {
        let mut policy = RecorderPolicy::default();
        policy.update(Some("a"), true);
        assert_eq!(policy.update(Some("a"), false), None);
        policy.set_automatic(false);
        assert_eq!(policy.update(Some("a"), false), Some(Presentation::Hide));
        policy.set_automatic(true);
        assert_eq!(policy.update(Some("a"), false), Some(Presentation::Show));
        policy.dismiss();
        assert_eq!(policy.update(Some("a"), false), None);
        policy.set_automatic(true);
        assert_eq!(policy.update(Some("a"), false), Some(Presentation::Show));
    }

    #[test]
    fn failed_show_rolls_back_visibility_and_retries_only_after_a_transition() {
        let mut policy = RecorderPolicy::default();
        assert_eq!(policy.update(Some("a"), true), Some(Presentation::Show));
        policy.show_failed();
        assert!(!policy.shown);
        assert_eq!(policy.update(Some("a"), true), None);
        assert_eq!(policy.update(Some("a"), true), None);
        assert_eq!(policy.update(Some("a"), false), Some(Presentation::Show));
        assert_eq!(policy.update(Some("a"), true), None);
        policy.show_failed();
        policy.set_automatic(true);
        assert_eq!(policy.update(Some("a"), true), Some(Presentation::Show));
        policy.show_failed();
        assert_eq!(policy.update(Some("b"), true), Some(Presentation::Show));
    }
}
