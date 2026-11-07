use super::{validate_name, MeetingAttendee, PeopleCatalog, Person};
use crate::{engine::Engine, store, CoreError, CoreHandle, Meeting, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum GoogleAttendeeOrigin {
    CalendarInvitation,
    MeetParticipant,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum GoogleAttendeeKind {
    Person,
    CalendarResource,
    SignedInPersonOrRoom,
    Anonymous,
    Phone,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GoogleAttendeeSource {
    pub source_key: String,
    pub origin: GoogleAttendeeOrigin,
    pub kind: GoogleAttendeeKind,
    pub display_name: Option<String>,
    pub email: Option<String>,
    pub google_user_id: Option<String>,
    pub response_status: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GoogleIdentityLink {
    pub person_id: String,
    pub emails: Vec<String>,
    pub google_user_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImportedGoogleAttendee {
    pub attendee_id: String,
    pub source: GoogleAttendeeSource,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GoogleAttendeeRoster {
    pub edit_version: String,
    pub meeting: Meeting,
    pub sources: Vec<ImportedGoogleAttendee>,
    pub identity_links: Vec<GoogleIdentityLink>,
    pub catalog: PeopleCatalog,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GoogleAttendeePersonLink {
    pub attendee_id: String,
    pub person_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GoogleAttendeePromotion {
    pub attendee_id: String,
    pub person: Person,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GoogleAttendeeDraft {
    pub attendees: Vec<MeetingAttendee>,
    pub sources: Vec<ImportedGoogleAttendee>,
    pub person_links: Vec<GoogleAttendeePersonLink>,
    pub promotions: Vec<GoogleAttendeePromotion>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct GoogleIdentityState {
    #[serde(default)]
    pub links: Vec<GoogleIdentityLink>,
    #[serde(default)]
    pub meetings: BTreeMap<String, Vec<ImportedGoogleAttendee>>,
}

fn invalid() -> CoreError {
    CoreError::InvalidInput("Google 참석자 정보를 확인해 주세요.".into())
}

fn safe_value(value: &str, maximum: usize) -> bool {
    !value.is_empty()
        && value == value.trim()
        && value.len() <= maximum
        && !value.chars().any(char::is_control)
}

pub(crate) fn normalize_email(value: &str) -> Option<String> {
    let value = value.trim();
    if !safe_value(value, 320) || value.chars().any(char::is_whitespace) {
        return None;
    }
    let (local, domain) = value.split_once('@')?;
    if local.is_empty() || domain.is_empty() || domain.contains('@') {
        return None;
    }
    Some(value.to_ascii_lowercase())
}

pub(crate) fn valid_google_user_id(value: &str) -> bool {
    value.strip_prefix("users/").is_some_and(|id| {
        !id.is_empty()
            && id.len() <= 256
            && id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    })
}

impl GoogleAttendeeSource {
    pub fn validate(&self) -> Result<()> {
        if !safe_value(&self.source_key, 2048)
            || self
                .display_name
                .as_ref()
                .is_some_and(|name| validate_name(name).is_err())
            || self
                .email
                .as_ref()
                .is_some_and(|email| normalize_email(email).as_ref() != Some(email))
            || self
                .google_user_id
                .as_ref()
                .is_some_and(|id| !valid_google_user_id(id))
            || self.response_status.as_ref().is_some_and(|status| {
                !matches!(
                    status.as_str(),
                    "needsAction" | "declined" | "tentative" | "accepted"
                )
            })
        {
            return Err(invalid());
        }
        match self.origin {
            GoogleAttendeeOrigin::CalendarInvitation => {
                if !matches!(
                    self.kind,
                    GoogleAttendeeKind::Person
                        | GoogleAttendeeKind::CalendarResource
                        | GoogleAttendeeKind::Unknown
                ) || self.google_user_id.is_some()
                {
                    return Err(invalid());
                }
            }
            GoogleAttendeeOrigin::MeetParticipant => {
                if matches!(
                    self.kind,
                    GoogleAttendeeKind::Person | GoogleAttendeeKind::CalendarResource
                ) || self.response_status.is_some()
                    || self.email.is_some()
                    || (self.google_user_id.is_some()
                        != matches!(self.kind, GoogleAttendeeKind::SignedInPersonOrRoom))
                {
                    return Err(invalid());
                }
            }
        }
        Ok(())
    }

    fn same_identity(&self, other: &Self) -> bool {
        self.email
            .as_ref()
            .is_some_and(|email| other.email.as_ref() == Some(email))
            || self
                .google_user_id
                .as_ref()
                .is_some_and(|id| other.google_user_id.as_ref() == Some(id))
    }

    fn matches(&self, link: &GoogleIdentityLink) -> bool {
        self.email
            .as_ref()
            .is_some_and(|email| link.emails.contains(email))
            || self
                .google_user_id
                .as_ref()
                .is_some_and(|id| link.google_user_ids.contains(id))
    }

    fn default_name(&self) -> String {
        self.display_name
            .clone()
            .or_else(|| {
                self.email
                    .clone()
                    .filter(|email| email.chars().count() <= 120)
            })
            .unwrap_or_else(|| {
                match self.kind {
                    GoogleAttendeeKind::Anonymous => "익명 참석자",
                    GoogleAttendeeKind::Phone => "전화 참석자",
                    GoogleAttendeeKind::CalendarResource => "캘린더 리소스",
                    _ => "Google 참석자",
                }
                .into()
            })
    }
}

impl GoogleIdentityState {
    // Adds these Google identities to the person's link, refusing any already linked
    // to someone else.
    fn link(&mut self, person_id: &str, sources: &[&GoogleAttendeeSource]) -> Result<()> {
        if self.links.iter().any(|link| {
            link.person_id != person_id && sources.iter().any(|source| source.matches(link))
        }) {
            return Err(CoreError::InvalidInput(
                "이미 다른 관리 인원에 연결된 Google 신원입니다.".into(),
            ));
        }
        let mut link = self
            .links
            .iter()
            .find(|link| link.person_id == person_id)
            .cloned()
            .unwrap_or(GoogleIdentityLink {
                person_id: person_id.into(),
                emails: vec![],
                google_user_ids: vec![],
            });
        for source in sources {
            if let Some(email) = &source.email {
                if !link.emails.contains(email) {
                    link.emails.push(email.clone());
                }
            }
            if let Some(user_id) = &source.google_user_id {
                if !link.google_user_ids.contains(user_id) {
                    link.google_user_ids.push(user_id.clone());
                }
            }
        }
        if !link.emails.is_empty() || !link.google_user_ids.is_empty() {
            link.emails.sort();
            link.google_user_ids.sort();
            self.links.retain(|old| old.person_id != person_id);
            self.links.push(link);
        }
        Ok(())
    }

    pub fn is_empty(&self) -> bool {
        self.links.is_empty() && self.meetings.is_empty()
    }

    pub fn validate(&self) -> Result<()> {
        let mut identities = BTreeSet::new();
        let mut people = BTreeSet::new();
        for link in &self.links {
            store::validate_id(&link.person_id)?;
            if !people.insert(&link.person_id)
                || (link.emails.is_empty() && link.google_user_ids.is_empty())
            {
                return Err(invalid());
            }
            for email in &link.emails {
                if normalize_email(email).as_ref() != Some(email)
                    || !identities.insert(("email", email))
                {
                    return Err(invalid());
                }
            }
            for id in &link.google_user_ids {
                if !valid_google_user_id(id) || !identities.insert(("user", id)) {
                    return Err(invalid());
                }
            }
        }
        for (meeting_id, entries) in &self.meetings {
            store::validate_id(meeting_id)?;
            let mut sources = BTreeSet::new();
            for entry in entries {
                store::validate_id(&entry.attendee_id)?;
                entry.source.validate()?;
                if !sources.insert(&entry.source.source_key) {
                    return Err(invalid());
                }
            }
        }
        Ok(())
    }
}

impl Engine {
    pub fn google_attendee_roster(&self, id: &str) -> Result<GoogleAttendeeRoster> {
        self.require_active_meeting(id)?;
        let meeting = self.meetings[id].meeting.clone();
        let sources = self
            .library
            .google_identity
            .meetings
            .get(id)
            .cloned()
            .unwrap_or_default();
        let edit_version = crate::edit_version::fingerprint(&(
            crate::edit_version::meeting_edit_version(&meeting)?,
            &self.library.google_identity,
            self.people_catalog()?.edit_version,
        ))?;
        Ok(GoogleAttendeeRoster {
            edit_version,
            meeting,
            sources,
            identity_links: self.library.google_identity.links.clone(),
            catalog: self.people_catalog()?,
        })
    }

    pub fn apply_meeting_attendee_draft(
        &mut self,
        id: &str,
        expected_version: &str,
        draft: GoogleAttendeeDraft,
    ) -> Result<GoogleAttendeeRoster> {
        let current = self.google_attendee_roster(id)?;
        if current.edit_version != expected_version {
            return Err(CoreError::EditConflict);
        }
        super::validate_attendees(&draft.attendees)?;
        let mut library = self.library.clone();
        let mut promoted = BTreeSet::new();
        for promotion in &draft.promotions {
            store::validate_id(&promotion.person.id)?;
            validate_name(&promotion.person.name)?;
            if !promoted.insert(&promotion.person.id)
                || library
                    .people
                    .people
                    .iter()
                    .any(|person| person.id == promotion.person.id)
                || !draft.attendees.iter().any(|attendee| {
                    attendee.id == promotion.attendee_id
                        && attendee.person_id.as_ref() == Some(&promotion.person.id)
                })
            {
                return Err(invalid());
            }
            library.people.people.push(promotion.person.clone());
        }
        super::require_known_people(
            &current.meeting.attendees,
            &draft.attendees,
            &library.people,
        )?;
        let mut records = current.sources;
        let mut keys = BTreeSet::new();
        for entry in draft.sources {
            store::validate_id(&entry.attendee_id)?;
            entry.source.validate()?;
            let retained = records.iter().any(|old| {
                old.source.source_key == entry.source.source_key
                    && old.attendee_id == entry.attendee_id
            });
            if !keys.insert(entry.source.source_key.clone())
                || (!retained
                    && !draft
                        .attendees
                        .iter()
                        .any(|attendee| attendee.id == entry.attendee_id))
            {
                return Err(invalid());
            }
            if let Some(old) = records
                .iter_mut()
                .find(|old| old.source.source_key == entry.source.source_key)
            {
                *old = entry;
            } else {
                records.push(entry);
            }
        }
        let mut linked = BTreeSet::new();
        for requested in draft.person_links {
            if !linked.insert(requested.attendee_id.clone())
                || !library
                    .people
                    .people
                    .iter()
                    .any(|person| person.id == requested.person_id)
                || !draft.attendees.iter().any(|attendee| {
                    attendee.id == requested.attendee_id
                        && attendee.person_id.as_ref() == Some(&requested.person_id)
                })
            {
                return Err(invalid());
            }
            let sources = records
                .iter()
                .filter(|entry| entry.attendee_id == requested.attendee_id)
                .map(|entry| &entry.source)
                .collect::<Vec<_>>();
            if sources.is_empty() {
                return Err(invalid());
            }
            library
                .google_identity
                .link(&requested.person_id, &sources)?;
        }
        if draft.attendees.is_empty() {
            library.meeting_attendees.remove(id);
        } else {
            library.meeting_attendees.insert(id.into(), draft.attendees);
        }
        if !records.is_empty() {
            library.google_identity.meetings.insert(id.into(), records);
        }
        self.commit_library(library)?;
        self.google_attendee_roster(id)
    }

    pub fn import_google_attendees(
        &mut self,
        id: &str,
        expected_version: &str,
        sources: Vec<GoogleAttendeeSource>,
    ) -> Result<GoogleAttendeeRoster> {
        let roster = self.google_attendee_roster(id)?;
        if roster.edit_version != expected_version {
            return Err(CoreError::EditConflict);
        }
        let mut unique = BTreeSet::new();
        for source in &sources {
            source.validate()?;
            if !unique.insert(&source.source_key) {
                return Err(invalid());
            }
        }
        let mut library = self.library.clone();
        let mut attendees = roster.meeting.attendees;
        let mut records = roster.sources;
        for source in sources {
            if let Some(record) = records
                .iter_mut()
                .find(|record| record.source.source_key == source.source_key)
            {
                record.source = source;
                continue;
            }
            let mut person_ids = library
                .google_identity
                .links
                .iter()
                .filter(|link| {
                    source.matches(link)
                        && library
                            .people
                            .people
                            .iter()
                            .any(|person| person.id == link.person_id)
                })
                .map(|link| link.person_id.clone())
                .collect::<BTreeSet<_>>();
            if person_ids.len() > 1 {
                return Err(CoreError::InvalidInput(
                    "Google 신원이 서로 다른 관리 인원에 연결되어 있습니다.".into(),
                ));
            }
            let person_id = person_ids.pop_first();
            let existing = records
                .iter()
                .find(|record| source.same_identity(&record.source))
                .and_then(|record| {
                    attendees
                        .iter()
                        .find(|attendee| attendee.id == record.attendee_id)
                })
                .or_else(|| {
                    person_id.as_ref().and_then(|person_id| {
                        attendees
                            .iter()
                            .find(|attendee| attendee.person_id.as_ref() == Some(person_id))
                    })
                });
            let attendee_id = if let Some(existing) = existing {
                existing.id.clone()
            } else {
                let name = person_id
                    .as_ref()
                    .and_then(|id| {
                        library
                            .people
                            .people
                            .iter()
                            .find(|person| &person.id == id)
                            .map(|person| person.name.clone())
                    })
                    .unwrap_or_else(|| source.default_name());
                let attendee_id = uuid::Uuid::new_v4().to_string();
                attendees.push(MeetingAttendee {
                    id: attendee_id.clone(),
                    person_id,
                    name: Some(name),
                });
                attendee_id
            };
            records.push(ImportedGoogleAttendee {
                attendee_id,
                source,
            });
        }
        super::validate_attendees(&attendees)?;
        if !attendees.is_empty() {
            library.meeting_attendees.insert(id.into(), attendees);
        }
        if !records.is_empty() {
            library.google_identity.meetings.insert(id.into(), records);
        }
        self.commit_library(library)?;
        self.google_attendee_roster(id)
    }

    pub fn attach_google_attendee_source(
        &mut self,
        id: &str,
        expected_version: &str,
        attendee_id: &str,
        source: GoogleAttendeeSource,
    ) -> Result<GoogleAttendeeRoster> {
        let roster = self.google_attendee_roster(id)?;
        if roster.edit_version != expected_version {
            return Err(CoreError::EditConflict);
        }
        source.validate()?;
        let target = roster
            .meeting
            .attendees
            .iter()
            .find(|attendee| attendee.id == attendee_id)
            .ok_or_else(invalid)?;
        let merged_ids = roster
            .sources
            .iter()
            .filter(|entry| {
                entry.source.source_key == source.source_key || entry.source.same_identity(&source)
            })
            .map(|entry| entry.attendee_id.clone())
            .filter(|source_id| source_id != attendee_id)
            .collect::<BTreeSet<_>>();
        for source_id in &merged_ids {
            if roster
                .meeting
                .attendees
                .iter()
                .find(|attendee| &attendee.id == source_id)
                .is_some_and(|old| old.person_id.is_some() && old.person_id != target.person_id)
            {
                return Err(CoreError::InvalidInput(
                    "이미 다른 관리 인원에 연결된 Google 참석자입니다.".into(),
                ));
            }
        }
        let mut library = self.library.clone();
        let attendees = library.meeting_attendees.get_mut(id).ok_or_else(invalid)?;
        attendees.retain(|attendee| !merged_ids.contains(&attendee.id));
        let records = library
            .google_identity
            .meetings
            .entry(id.into())
            .or_default();
        for record in records.iter_mut() {
            if merged_ids.contains(&record.attendee_id) {
                record.attendee_id = attendee_id.into();
            }
        }
        if let Some(record) = records
            .iter_mut()
            .find(|record| record.source.source_key == source.source_key)
        {
            record.attendee_id = attendee_id.into();
            record.source = source;
        } else {
            records.push(ImportedGoogleAttendee {
                attendee_id: attendee_id.into(),
                source,
            });
        }
        self.commit_library(library)?;
        self.google_attendee_roster(id)
    }

    pub fn link_google_attendee_person(
        &mut self,
        id: &str,
        expected_version: &str,
        attendee_id: &str,
        person_id: &str,
    ) -> Result<GoogleAttendeeRoster> {
        let roster = self.google_attendee_roster(id)?;
        if roster.edit_version != expected_version {
            return Err(CoreError::EditConflict);
        }
        let person = self
            .library
            .people
            .people
            .iter()
            .find(|person| person.id == person_id)
            .ok_or_else(|| CoreError::InvalidInput("관리 인원을 찾을 수 없습니다.".into()))?
            .clone();
        let selected = roster
            .meeting
            .attendees
            .iter()
            .find(|attendee| attendee.id == attendee_id)
            .ok_or_else(invalid)?;
        if selected
            .person_id
            .as_ref()
            .is_some_and(|id| id != person_id)
        {
            return Err(CoreError::InvalidInput(
                "이미 다른 관리 인원에 연결된 참석자입니다.".into(),
            ));
        }
        let source_entries = roster
            .sources
            .iter()
            .filter(|entry| entry.attendee_id == attendee_id)
            .collect::<Vec<_>>();
        if source_entries.is_empty() {
            return Err(invalid());
        }
        let mut library = self.library.clone();
        let sources = source_entries
            .iter()
            .map(|entry| &entry.source)
            .collect::<Vec<_>>();
        library.google_identity.link(person_id, &sources)?;
        let target = roster
            .meeting
            .attendees
            .iter()
            .find(|attendee| attendee.person_id.as_deref() == Some(person_id))
            .map(|attendee| attendee.id.clone())
            .unwrap_or_else(|| attendee_id.into());
        let attendees = library.meeting_attendees.get_mut(id).ok_or_else(invalid)?;
        if target != attendee_id {
            attendees.retain(|attendee| attendee.id != attendee_id);
        }
        let attendee = attendees
            .iter_mut()
            .find(|attendee| attendee.id == target)
            .ok_or_else(invalid)?;
        attendee.person_id = Some(person.id);
        // An explicit roster name is a meeting snapshot, independent of catalog renames.
        if attendee.name.is_none() {
            attendee.name = Some(person.name);
        }
        for record in library
            .google_identity
            .meetings
            .get_mut(id)
            .into_iter()
            .flatten()
        {
            if record.attendee_id == attendee_id {
                record.attendee_id = target.clone();
            }
        }
        self.commit_library(library)?;
        self.google_attendee_roster(id)
    }
}

impl CoreHandle {
    pub fn apply_meeting_attendee_draft(
        &self,
        id: &str,
        expected_version: &str,
        draft: GoogleAttendeeDraft,
    ) -> Result<GoogleAttendeeRoster> {
        let (id, version) = (id.to_owned(), expected_version.to_owned());
        self.call(move |engine| engine.apply_meeting_attendee_draft(&id, &version, draft))
    }
    pub fn google_attendee_roster(&self, id: &str) -> Result<GoogleAttendeeRoster> {
        let id = id.to_owned();
        self.call(move |engine| engine.google_attendee_roster(&id))
    }
    pub fn import_google_attendees(
        &self,
        id: &str,
        expected_version: &str,
        sources: Vec<GoogleAttendeeSource>,
    ) -> Result<GoogleAttendeeRoster> {
        let (id, version) = (id.to_owned(), expected_version.to_owned());
        self.call(move |engine| engine.import_google_attendees(&id, &version, sources))
    }
    pub fn link_google_attendee_person(
        &self,
        id: &str,
        expected_version: &str,
        attendee_id: &str,
        person_id: &str,
    ) -> Result<GoogleAttendeeRoster> {
        let (id, version, attendee_id, person_id) = (
            id.to_owned(),
            expected_version.to_owned(),
            attendee_id.to_owned(),
            person_id.to_owned(),
        );
        self.call(move |engine| {
            engine.link_google_attendee_person(&id, &version, &attendee_id, &person_id)
        })
    }
    pub fn attach_google_attendee_source(
        &self,
        id: &str,
        expected_version: &str,
        attendee_id: &str,
        source: GoogleAttendeeSource,
    ) -> Result<GoogleAttendeeRoster> {
        let (id, version, attendee_id) = (
            id.to_owned(),
            expected_version.to_owned(),
            attendee_id.to_owned(),
        );
        self.call(move |engine| {
            engine.attach_google_attendee_source(&id, &version, &attendee_id, source)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::people::Person;

    fn engine() -> (tempfile::TempDir, Engine) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap().join("library");
        let engine = Engine::open(root).unwrap();
        (temp, engine)
    }
    fn invitation(key: &str, email: &str) -> GoogleAttendeeSource {
        GoogleAttendeeSource {
            source_key: key.into(),
            origin: GoogleAttendeeOrigin::CalendarInvitation,
            kind: GoogleAttendeeKind::Person,
            display_name: Some("Same name".into()),
            email: Some(email.into()),
            google_user_id: None,
            response_status: Some("accepted".into()),
        }
    }
    fn participant(key: &str, user_id: Option<&str>) -> GoogleAttendeeSource {
        GoogleAttendeeSource {
            source_key: key.into(),
            origin: GoogleAttendeeOrigin::MeetParticipant,
            kind: if user_id.is_some() {
                GoogleAttendeeKind::SignedInPersonOrRoom
            } else {
                GoogleAttendeeKind::Anonymous
            },
            display_name: Some("Same name".into()),
            email: None,
            google_user_id: user_id.map(str::to_owned),
            response_status: None,
        }
    }
    fn import(
        engine: &mut Engine,
        id: &str,
        sources: Vec<GoogleAttendeeSource>,
    ) -> GoogleAttendeeRoster {
        let version = engine.google_attendee_roster(id).unwrap().edit_version;
        engine
            .import_google_attendees(id, &version, sources)
            .unwrap()
    }

    #[test]
    fn names_do_not_merge_invitees_signed_in_accounts_or_anonymous_attendees() {
        let (_temp, mut engine) = engine();
        let meeting = engine
            .create_meeting("Synthetic".into(), "".into())
            .unwrap();
        let roster = import(
            &mut engine,
            &meeting.id,
            vec![
                invitation("invite:a", "first@example.test"),
                invitation("invite:b", "second@example.test"),
                participant("meet:a", Some("users/100")),
                participant("meet:b", Some("users/200")),
                participant("meet:c", None),
                participant("meet:d", None),
            ],
        );
        assert_eq!(roster.meeting.attendees.len(), 6);
        assert_eq!(roster.sources.len(), 6);
        let repeated = import(
            &mut engine,
            &meeting.id,
            roster
                .sources
                .iter()
                .map(|entry| entry.source.clone())
                .collect(),
        );
        assert_eq!(repeated.meeting.attendees, roster.meeting.attendees);
        assert!(repeated
            .meeting
            .attendees
            .iter()
            .all(|attendee| attendee.person_id.is_none()));
    }

    #[test]
    fn explicit_existing_person_link_survives_reimport_manual_names_and_reopen() {
        let (temp, mut engine) = engine();
        let person = Person {
            id: uuid::Uuid::new_v4().to_string(),
            name: "Catalog name".into(),
        };
        let catalog = engine.people_catalog().unwrap();
        engine
            .replace_people_catalog(&catalog.edit_version, vec![person.clone()], vec![])
            .unwrap();
        let meeting = engine
            .create_meeting("Synthetic".into(), "Preserved notes".into())
            .unwrap();
        let invited = invitation("invite:a", "first@example.test");
        let actual = participant("meet:a", Some("users/100"));
        let roster = import(
            &mut engine,
            &meeting.id,
            vec![invited.clone(), actual.clone()],
        );
        let first = engine
            .link_google_attendee_person(
                &meeting.id,
                &roster.edit_version,
                &roster.sources[0].attendee_id,
                &person.id,
            )
            .unwrap();
        let linked = engine
            .link_google_attendee_person(
                &meeting.id,
                &first.edit_version,
                &roster.sources[1].attendee_id,
                &person.id,
            )
            .unwrap();
        assert_eq!(linked.meeting.attendees.len(), 1);
        assert_eq!(linked.identity_links[0].emails, ["first@example.test"]);
        assert_eq!(linked.identity_links[0].google_user_ids, ["users/100"]);
        assert_eq!(linked.sources[0].attendee_id, linked.sources[1].attendee_id);
        let mut edited = linked.meeting.attendees.clone();
        edited[0].name = Some("Manual meeting name".into());
        let version = crate::edit_version::meeting_edit_version(&linked.meeting).unwrap();
        engine
            .replace_meeting_attendees(&meeting.id, &version, edited.clone())
            .unwrap();
        let mut changed_source = invited;
        changed_source.display_name = Some("New provider name".into());
        changed_source.response_status = Some("declined".into());
        let repeated = import(&mut engine, &meeting.id, vec![changed_source, actual]);
        assert_eq!(repeated.meeting.attendees, edited);
        assert_eq!(repeated.meeting.notes, "Preserved notes");
        assert_eq!(
            repeated.sources[0].source.response_status.as_deref(),
            Some("declined")
        );
        let another = engine.create_meeting("Another".into(), "".into()).unwrap();
        let next = import(
            &mut engine,
            &another.id,
            vec![
                invitation("invite:new", "first@example.test"),
                participant("meet:new", Some("users/100")),
            ],
        );
        assert_eq!(next.meeting.attendees.len(), 1);
        assert_eq!(
            next.meeting.attendees[0].person_id.as_ref(),
            Some(&person.id)
        );
        assert_eq!(engine.meetings[&meeting.id].meeting.attendees, edited);
        let repeated = engine.google_attendee_roster(&meeting.id).unwrap();
        drop(engine);
        let root = temp.path().canonicalize().unwrap().join("library");
        let reopened = Engine::open(root.clone()).unwrap();
        assert_eq!(
            reopened.google_attendee_roster(&meeting.id).unwrap(),
            repeated
        );
        let reader = crate::reader::LibraryReader::open(root).unwrap();
        assert!(reader
            .query("people_catalog", serde_json::json!({}))
            .is_ok());
    }

    #[test]
    fn manual_removal_and_unavailable_source_lists_do_not_erase_or_resurrect_people() {
        let (_temp, mut engine) = engine();
        let meeting = engine
            .create_meeting("Synthetic".into(), "".into())
            .unwrap();
        let source = invitation("invite:a", "first@example.test");
        let roster = import(&mut engine, &meeting.id, vec![source.clone()]);
        let retained = import(&mut engine, &meeting.id, vec![]);
        assert_eq!(retained.meeting.attendees, roster.meeting.attendees);
        let version = crate::edit_version::meeting_edit_version(&retained.meeting).unwrap();
        engine
            .replace_meeting_attendees(&meeting.id, &version, vec![])
            .unwrap();
        let repeated = import(&mut engine, &meeting.id, vec![source]);
        assert!(repeated.meeting.attendees.is_empty());
    }

    #[test]
    fn stale_versions_and_conflicting_identity_links_are_rejected_atomically() {
        let (_temp, mut engine) = engine();
        let people = ["First", "Second"].map(|name| Person {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.into(),
        });
        let catalog = engine.people_catalog().unwrap();
        engine
            .replace_people_catalog(&catalog.edit_version, people.to_vec(), vec![])
            .unwrap();
        let meeting = engine
            .create_meeting("Synthetic".into(), "".into())
            .unwrap();
        let roster = import(
            &mut engine,
            &meeting.id,
            vec![invitation("invite:a", "first@example.test")],
        );
        let linked = engine
            .link_google_attendee_person(
                &meeting.id,
                &roster.edit_version,
                &roster.sources[0].attendee_id,
                &people[0].id,
            )
            .unwrap();
        assert!(matches!(
            engine.import_google_attendees(&meeting.id, &roster.edit_version, vec![]),
            Err(CoreError::EditConflict)
        ));
        assert!(engine
            .link_google_attendee_person(
                &meeting.id,
                &linked.edit_version,
                &linked.sources[0].attendee_id,
                &people[1].id
            )
            .is_err());
        assert_eq!(engine.google_attendee_roster(&meeting.id).unwrap(), linked);
    }

    #[test]
    fn reading_legacy_library_does_not_create_identity_storage() {
        let (temp, engine) = engine();
        assert!(engine.library.google_identity.is_empty());
        let bytes = serde_json::to_vec(&engine.library).unwrap();
        assert!(!String::from_utf8(bytes).unwrap().contains("googleIdentity"));
        assert!(!temp.path().join("library/googleIdentity").exists());
    }

    #[test]
    fn attaching_to_a_temporary_attendee_preserves_its_name_without_registering_a_person() {
        let (_temp, mut engine) = engine();
        let meeting = engine
            .create_meeting("Synthetic".into(), "".into())
            .unwrap();
        let target = MeetingAttendee {
            id: uuid::Uuid::new_v4().to_string(),
            person_id: None,
            name: Some("Manual temporary name".into()),
        };
        let version = crate::edit_version::meeting_edit_version(&meeting).unwrap();
        engine
            .replace_meeting_attendees(&meeting.id, &version, vec![target.clone()])
            .unwrap();
        let source = participant("meet:a", Some("users/100"));
        let roster = import(&mut engine, &meeting.id, vec![source.clone()]);
        assert_eq!(roster.meeting.attendees.len(), 2);
        let attached = engine
            .attach_google_attendee_source(
                &meeting.id,
                &roster.edit_version,
                &target.id,
                source.clone(),
            )
            .unwrap();
        assert_eq!(attached.meeting.attendees, [target.clone()]);
        assert!(attached.identity_links.is_empty());
        assert!(engine.people_catalog().unwrap().people.is_empty());
        let repeated = import(
            &mut engine,
            &meeting.id,
            vec![source, participant("meet:rejoined", Some("users/100"))],
        );
        assert_eq!(repeated.meeting.attendees, [target.clone()]);
        assert!(repeated
            .sources
            .iter()
            .all(|source| source.attendee_id == target.id));
        assert!(repeated.identity_links.is_empty());
    }

    #[test]
    fn linking_to_an_already_selected_person_preserves_the_existing_attendee_and_moves_all_sources()
    {
        let (_temp, mut engine) = engine();
        let person = Person {
            id: uuid::Uuid::new_v4().to_string(),
            name: "Catalog".into(),
        };
        let catalog = engine.people_catalog().unwrap();
        engine
            .replace_people_catalog(&catalog.edit_version, vec![person.clone()], vec![])
            .unwrap();
        let meeting = engine
            .create_meeting("Synthetic".into(), "".into())
            .unwrap();
        let target = MeetingAttendee {
            id: uuid::Uuid::new_v4().to_string(),
            person_id: Some(person.id.clone()),
            name: Some("Existing meeting name".into()),
        };
        let version = crate::edit_version::meeting_edit_version(&meeting).unwrap();
        engine
            .replace_meeting_attendees(&meeting.id, &version, vec![target.clone()])
            .unwrap();
        let roster = import(
            &mut engine,
            &meeting.id,
            vec![
                participant("meet:a", Some("users/100")),
                participant("meet:b", Some("users/100")),
            ],
        );
        assert_eq!(roster.meeting.attendees.len(), 2);
        let linked = engine
            .link_google_attendee_person(
                &meeting.id,
                &roster.edit_version,
                &roster.sources[0].attendee_id,
                &person.id,
            )
            .unwrap();
        assert_eq!(linked.meeting.attendees, [target.clone()]);
        assert!(linked
            .sources
            .iter()
            .all(|entry| entry.attendee_id == target.id));
        assert_eq!(linked.identity_links[0].google_user_ids, ["users/100"]);
    }

    #[test]
    fn one_draft_commit_applies_manual_names_google_links_and_person_promotion() {
        let (_temp, mut engine) = engine();
        let meeting = engine
            .create_meeting("Synthetic".into(), "Preserved notes".into())
            .unwrap();
        let current = engine.google_attendee_roster(&meeting.id).unwrap();
        let attendee_id = uuid::Uuid::new_v4().to_string();
        let person = Person {
            id: uuid::Uuid::new_v4().to_string(),
            name: "New saved person".into(),
        };
        let draft = GoogleAttendeeDraft {
            attendees: vec![MeetingAttendee {
                id: attendee_id.clone(),
                person_id: Some(person.id.clone()),
                name: Some("Manual meeting name".into()),
            }],
            sources: vec![
                ImportedGoogleAttendee {
                    attendee_id: attendee_id.clone(),
                    source: participant("meet:a", Some("users/100")),
                },
                ImportedGoogleAttendee {
                    attendee_id: attendee_id.clone(),
                    source: invitation("invite:a", "first@example.test"),
                },
            ],
            person_links: vec![GoogleAttendeePersonLink {
                attendee_id: attendee_id.clone(),
                person_id: person.id.clone(),
            }],
            promotions: vec![GoogleAttendeePromotion {
                attendee_id,
                person: person.clone(),
            }],
        };
        let result = engine
            .apply_meeting_attendee_draft(&meeting.id, &current.edit_version, draft.clone())
            .unwrap();
        assert_eq!(result.meeting.attendees, draft.attendees);
        assert_eq!(result.sources, draft.sources);
        assert_eq!(result.catalog.people, [person]);
        assert_eq!(result.identity_links[0].emails, ["first@example.test"]);
        assert_eq!(result.identity_links[0].google_user_ids, ["users/100"]);
        assert_eq!(result.meeting.notes, "Preserved notes");
        assert!(matches!(
            engine.apply_meeting_attendee_draft(&meeting.id, &current.edit_version, draft),
            Err(CoreError::EditConflict)
        ));
    }

    #[test]
    fn a_failed_draft_identity_link_does_not_save_prior_manual_edits_or_person_promotion() {
        let (_temp, mut engine) = engine();
        let existing = Person {
            id: uuid::Uuid::new_v4().to_string(),
            name: "Existing saved person".into(),
        };
        let catalog = engine.people_catalog().unwrap();
        engine
            .replace_people_catalog(&catalog.edit_version, vec![existing.clone()], vec![])
            .unwrap();
        let meeting = engine
            .create_meeting("Synthetic".into(), "Preserved notes".into())
            .unwrap();
        let imported = import(
            &mut engine,
            &meeting.id,
            vec![participant("meet:a", Some("users/100"))],
        );
        let current = engine
            .link_google_attendee_person(
                &meeting.id,
                &imported.edit_version,
                &imported.sources[0].attendee_id,
                &existing.id,
            )
            .unwrap();
        let before = std::fs::read(engine.store.root.join("library.json")).unwrap();
        let attendee_id = uuid::Uuid::new_v4().to_string();
        let person = Person {
            id: uuid::Uuid::new_v4().to_string(),
            name: "Uncommitted person".into(),
        };
        let draft = GoogleAttendeeDraft {
            attendees: vec![MeetingAttendee {
                id: attendee_id.clone(),
                person_id: Some(person.id.clone()),
                name: Some("Uncommitted manual name".into()),
            }],
            sources: vec![ImportedGoogleAttendee {
                attendee_id: attendee_id.clone(),
                source: participant("meet:a", Some("users/100")),
            }],
            person_links: vec![GoogleAttendeePersonLink {
                attendee_id: attendee_id.clone(),
                person_id: person.id.clone(),
            }],
            promotions: vec![GoogleAttendeePromotion {
                attendee_id,
                person,
            }],
        };
        assert!(engine
            .apply_meeting_attendee_draft(&meeting.id, &current.edit_version, draft)
            .is_err());
        assert_eq!(engine.google_attendee_roster(&meeting.id).unwrap(), current);
        assert_eq!(
            std::fs::read(engine.store.root.join("library.json")).unwrap(),
            before
        );
    }

    #[test]
    fn deleting_a_person_releases_global_identity_without_rewriting_old_meetings() {
        let (_temp, mut engine) = engine();
        let person = Person {
            id: uuid::Uuid::new_v4().to_string(),
            name: "First saved person".into(),
        };
        let replacement = Person {
            id: uuid::Uuid::new_v4().to_string(),
            name: "Replacement person".into(),
        };
        let catalog = engine.people_catalog().unwrap();
        engine
            .replace_people_catalog(
                &catalog.edit_version,
                vec![person.clone(), replacement.clone()],
                vec![],
            )
            .unwrap();
        let first = engine
            .create_meeting("First synthetic".into(), "".into())
            .unwrap();
        let roster = import(
            &mut engine,
            &first.id,
            vec![participant("meet:first", Some("users/100"))],
        );
        let linked = engine
            .link_google_attendee_person(
                &first.id,
                &roster.edit_version,
                &roster.sources[0].attendee_id,
                &person.id,
            )
            .unwrap();
        let catalog = engine.people_catalog().unwrap();
        engine
            .replace_people_catalog(&catalog.edit_version, vec![replacement.clone()], vec![])
            .unwrap();
        let preserved = engine.google_attendee_roster(&first.id).unwrap();
        assert_eq!(preserved.meeting.attendees, linked.meeting.attendees);
        assert_eq!(preserved.sources, linked.sources);
        assert!(preserved.identity_links.is_empty());
        let second = engine
            .create_meeting("Second synthetic".into(), "".into())
            .unwrap();
        let roster = import(
            &mut engine,
            &second.id,
            vec![participant("meet:second", Some("users/100"))],
        );
        let linked = engine
            .link_google_attendee_person(
                &second.id,
                &roster.edit_version,
                &roster.sources[0].attendee_id,
                &replacement.id,
            )
            .unwrap();
        assert_eq!(linked.identity_links[0].person_id, replacement.id);
        assert_eq!(
            engine
                .google_attendee_roster(&first.id)
                .unwrap()
                .meeting
                .attendees,
            preserved.meeting.attendees
        );
    }
}
