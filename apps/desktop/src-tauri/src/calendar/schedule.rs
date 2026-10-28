use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const LEAD_MS: u64 = 5 * 60 * 1000;
const RETAIN_MS: u64 = 24 * 60 * 60 * 1000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Occurrence {
    pub key: String,
    pub title: String,
    pub start_at: u64,
    pub end_at: u64,
    pub calendar_name: String,
    pub join_urls: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReminderChange {
    pub kind: &'static str,
    pub key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event: Option<Occurrence>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Notice {
    pub retain_until: u64,
    pub dismissed: bool,
}

#[derive(Default, Clone)]
pub struct Scheduler {
    pub notices: BTreeMap<String, Notice>,
    pub(super) visible: BTreeMap<String, Occurrence>,
}

impl Scheduler {
    pub fn tick(
        &mut self,
        events: &[Occurrence],
        now: u64,
        fresh: bool,
        recording: bool,
    ) -> Vec<ReminderChange> {
        self.notices.retain(|_, notice| notice.retain_until > now);
        let candidates = events
            .iter()
            .filter(|event| {
                fresh
                    && !recording
                    && event.start_at > now
                    && event.end_at > event.start_at
                    && now >= event.start_at.saturating_sub(LEAD_MS)
            })
            .map(|event| (event.key.clone(), event.clone()))
            .collect::<BTreeMap<_, _>>();
        let mut changes = Vec::new();
        for key in self
            .visible
            .keys()
            .filter(|key| !candidates.contains_key(*key))
            .cloned()
            .collect::<Vec<_>>()
        {
            self.visible.remove(&key);
            changes.push(ReminderChange {
                kind: "hide",
                key,
                event: None,
            });
        }
        for (key, event) in candidates {
            if let Some(visible) = self.visible.get(&key) {
                if visible != &event {
                    self.visible.insert(key.clone(), event.clone());
                    changes.push(ReminderChange {
                        kind: "update",
                        key,
                        event: Some(event),
                    });
                }
            } else if self
                .notices
                .get(&key)
                .is_none_or(|notice| !notice.dismissed)
            {
                self.notices.insert(
                    key.clone(),
                    Notice {
                        retain_until: event.end_at.saturating_add(RETAIN_MS),
                        dismissed: false,
                    },
                );
                self.visible.insert(key.clone(), event.clone());
                changes.push(ReminderChange {
                    kind: "show",
                    key,
                    event: Some(event),
                });
            }
        }
        changes
    }

    pub fn dismiss(&mut self, key: &str) -> Option<ReminderChange> {
        if let Some(notice) = self.notices.get_mut(key) {
            notice.dismissed = true;
        }
        self.visible.remove(key).map(|_| ReminderChange {
            kind: "hide",
            key: key.into(),
            event: None,
        })
    }

    pub fn visible(&self, key: &str) -> Option<Occurrence> {
        self.visible.get(key).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn event(key: &str, start: u64) -> Occurrence {
        Occurrence {
            key: key.into(),
            title: "Synthetic meeting".into(),
            start_at: start,
            end_at: start + 600_000,
            calendar_name: "Synthetic calendar".into(),
            join_urls: vec![],
        }
    }
    #[test]
    fn alerts_once_in_five_minute_window_and_never_after_start_or_when_stale() {
        let item = event("first", 900_000);
        let mut scheduler = Scheduler::default();
        assert!(scheduler
            .tick(&[item.clone()], 599_999, true, false)
            .is_empty());
        assert!(scheduler
            .tick(&[item.clone()], 600_000, false, false)
            .is_empty());
        assert!(scheduler
            .tick(&[item.clone()], 600_000, true, true)
            .is_empty());
        assert_eq!(
            scheduler.tick(&[item.clone()], 600_000, true, false)[0].kind,
            "show"
        );
        assert!(scheduler
            .tick(&[item.clone()], 600_001, true, false)
            .is_empty());
        assert_eq!(
            scheduler.tick(&[item.clone()], 900_000, true, false)[0].kind,
            "hide"
        );
        assert!(scheduler.tick(&[item], 900_001, true, false).is_empty());
        assert!(scheduler
            .tick(&[event("missed", 700_000)], 900_001, true, false)
            .is_empty());
    }
    #[test]
    fn changed_cancelled_dismissed_and_restarted_occurrences_do_not_duplicate() {
        let original = event("stable-occurrence", 900_000);
        let mut scheduler = Scheduler::default();
        scheduler.tick(&[original.clone()], 600_000, true, false);
        let mut changed = original.clone();
        changed.title = "Changed title".into();
        changed.start_at += 60_000;
        assert_eq!(
            scheduler.tick(&[changed.clone()], 660_000, true, false)[0].kind,
            "update"
        );
        assert_eq!(scheduler.dismiss(&changed.key).unwrap().kind, "hide");
        assert!(scheduler
            .tick(&[changed.clone()], 660_001, true, false)
            .is_empty());
        let mut restarted = Scheduler {
            notices: scheduler.notices,
            ..Scheduler::default()
        };
        assert!(restarted.tick(&[changed], 660_002, true, false).is_empty());
        let next = event("next-repeat-instance", 1_000_000);
        assert_eq!(
            restarted.tick(&[next], 700_000, true, false)[0].kind,
            "show"
        );
        assert_eq!(restarted.tick(&[], 700_001, true, false)[0].kind, "hide");
    }
    #[test]
    fn a_visible_notice_returns_after_a_fresh_wake_without_repeating_a_dismissal() {
        let item = event("current-occurrence", 900_000);
        let mut scheduler = Scheduler::default();
        assert_eq!(
            scheduler.tick(&[item.clone()], 600_000, true, false)[0].kind,
            "show"
        );
        assert!(scheduler
            .tick(&[item.clone()], 600_001, true, false)
            .is_empty());
        assert_eq!(
            scheduler.tick(&[item.clone()], 610_000, false, false)[0].kind,
            "hide"
        );
        assert_eq!(
            scheduler.tick(&[item.clone()], 610_001, true, false)[0].kind,
            "show"
        );
        scheduler.dismiss(&item.key);
        assert!(scheduler.tick(&[item], 610_002, true, false).is_empty());
    }
}
