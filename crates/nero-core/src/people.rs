use crate::{agent_control, engine::Engine, store, CoreError, CoreHandle, Meeting, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Person {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonGroup {
    pub id: String,
    pub name: String,
    pub person_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MeetingAttendee {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub person_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PeopleCatalog {
    pub edit_version: String,
    pub people: Vec<Person>,
    pub groups: Vec<PersonGroup>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PromotedAttendee {
    pub meeting: Meeting,
    pub catalog: PeopleCatalog,
    pub person: Person,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct People {
    pub people: Vec<Person>,
    pub groups: Vec<PersonGroup>,
}

fn validate_name(name: &str) -> Result<()> {
    if name.trim().is_empty()
        || name != name.trim()
        || name.chars().count() > 120
        || name.chars().any(char::is_control)
    {
        return Err(CoreError::InvalidInput(
            "이름은 앞뒤 공백 없이 1자에서 120자 사이로 입력해 주세요.".into(),
        ));
    }
    Ok(())
}

pub(crate) fn validate_attendees(attendees: &[MeetingAttendee]) -> Result<()> {
    let mut ids = BTreeSet::new();
    let mut people = BTreeSet::new();
    for attendee in attendees {
        store::validate_id(&attendee.id)?;
        if !ids.insert(&attendee.id) {
            return Err(CoreError::InvalidInput("중복된 참석자 ID입니다.".into()));
        }
        if let Some(id) = &attendee.person_id {
            store::validate_id(id)?;
            if !people.insert(id) || attendee.name.is_none() {
                return Err(CoreError::InvalidInput(
                    "관리 인원 연결이 중복되었거나 이름이 없습니다.".into(),
                ));
            }
        }
        if let Some(name) = &attendee.name {
            validate_name(name)?;
        }
    }
    Ok(())
}

pub fn attendees_digest(attendees: &[MeetingAttendee]) -> Result<String> {
    validate_attendees(attendees)?;
    let bytes = serde_json::to_vec(attendees).map_err(|e| CoreError::Storage(e.to_string()))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

impl People {
    pub(crate) fn validate(&self) -> Result<()> {
        let mut ids = BTreeSet::new();
        for person in &self.people {
            store::validate_id(&person.id)?;
            validate_name(&person.name)?;
            if !ids.insert(&person.id) {
                return Err(CoreError::InvalidInput("중복된 관리 인원 ID입니다.".into()));
            }
        }
        let mut group_ids = BTreeSet::new();
        let mut names = BTreeSet::new();
        for group in &self.groups {
            store::validate_id(&group.id)?;
            validate_name(&group.name)?;
            let mut members = BTreeSet::new();
            if !group_ids.insert(&group.id)
                || !names.insert(group.name.to_lowercase())
                || group
                    .person_ids
                    .iter()
                    .any(|id| !ids.contains(id) || !members.insert(id))
            {
                return Err(CoreError::InvalidInput(
                    "그룹 또는 그룹 구성원이 중복되었거나 존재하지 않습니다.".into(),
                ));
            }
        }
        Ok(())
    }

    pub(crate) fn catalog(&self) -> Result<PeopleCatalog> {
        let bytes = serde_json::to_vec(self).map_err(|e| CoreError::Storage(e.to_string()))?;
        Ok(PeopleCatalog {
            edit_version: format!("sha256:{:x}", Sha256::digest(bytes)),
            people: self.people.clone(),
            groups: self.groups.clone(),
        })
    }
}

impl Engine {
    pub fn people_catalog(&self) -> Result<PeopleCatalog> {
        self.library.people.catalog()
    }

    pub fn replace_people_catalog(
        &mut self,
        expected_version: &str,
        people: Vec<Person>,
        groups: Vec<PersonGroup>,
    ) -> Result<PeopleCatalog> {
        if self.people_catalog()?.edit_version != expected_version {
            return Err(CoreError::EditConflict);
        }
        let people = People { people, groups };
        people.validate()?;
        let mut library = self.library.clone();
        library.people = people;
        self.commit_library(library)?;
        self.people_catalog()
    }

    pub fn replace_meeting_attendees(
        &mut self,
        id: &str,
        expected_version: &str,
        attendees: Vec<MeetingAttendee>,
    ) -> Result<Meeting> {
        self.require_active_meeting(id)?;
        let meeting = &self.meetings[id].meeting;
        if agent_control::meeting_edit_version(meeting)? != expected_version {
            return Err(CoreError::EditConflict);
        }
        validate_attendees(&attendees)?;
        for attendee in &attendees {
            if let Some(person_id) = &attendee.person_id {
                let retained = meeting
                    .attendees
                    .iter()
                    .any(|old| old.id == attendee.id && old.person_id.as_ref() == Some(person_id));
                if !retained
                    && !self
                        .library
                        .people
                        .people
                        .iter()
                        .any(|person| &person.id == person_id)
                {
                    return Err(CoreError::InvalidInput(
                        "선택한 관리 인원을 찾을 수 없습니다.".into(),
                    ));
                }
            }
        }
        let mut library = self.library.clone();
        if attendees.is_empty() {
            library.meeting_attendees.remove(id);
        } else {
            library.meeting_attendees.insert(id.into(), attendees);
        }
        self.commit_library(library)?;
        Ok(self.meetings[id].meeting.clone())
    }

    pub fn promote_meeting_attendee(
        &mut self,
        id: &str,
        expected_version: &str,
        expected_catalog_version: &str,
        attendee_id: &str,
        name: String,
    ) -> Result<PromotedAttendee> {
        self.require_active_meeting(id)?;
        store::validate_id(attendee_id)?;
        validate_name(&name)?;
        if agent_control::meeting_edit_version(&self.meetings[id].meeting)? != expected_version
            || self.people_catalog()?.edit_version != expected_catalog_version
        {
            return Err(CoreError::EditConflict);
        }
        let mut library = self.library.clone();
        let attendee = library
            .meeting_attendees
            .get_mut(id)
            .and_then(|attendees| {
                attendees
                    .iter_mut()
                    .find(|attendee| attendee.id == attendee_id)
            })
            .ok_or_else(|| CoreError::InvalidInput("참석자를 찾을 수 없습니다.".into()))?;
        if attendee.person_id.is_some() {
            return Err(CoreError::InvalidInput(
                "이미 관리 인원에 연결된 참석자입니다.".into(),
            ));
        }
        let person = Person {
            id: uuid::Uuid::new_v4().to_string(),
            name,
        };
        attendee.person_id = Some(person.id.clone());
        attendee.name = Some(person.name.clone());
        library.people.people.push(person.clone());
        self.commit_library(library)?;
        Ok(PromotedAttendee {
            meeting: self.meetings[id].meeting.clone(),
            catalog: self.people_catalog()?,
            person,
        })
    }
}

impl CoreHandle {
    pub fn people_catalog(&self) -> Result<PeopleCatalog> {
        self.call(|engine| engine.people_catalog())
    }
    pub fn replace_people_catalog(
        &self,
        expected_version: &str,
        people: Vec<Person>,
        groups: Vec<PersonGroup>,
    ) -> Result<PeopleCatalog> {
        let version = expected_version.to_owned();
        self.call(move |engine| engine.replace_people_catalog(&version, people, groups))
    }
    pub fn replace_meeting_attendees(
        &self,
        id: &str,
        expected_version: &str,
        attendees: Vec<MeetingAttendee>,
    ) -> Result<Meeting> {
        let (id, version) = (id.to_owned(), expected_version.to_owned());
        self.call(move |engine| engine.replace_meeting_attendees(&id, &version, attendees))
    }
    pub fn promote_meeting_attendee(
        &self,
        id: &str,
        expected_version: &str,
        expected_catalog_version: &str,
        attendee_id: &str,
        name: String,
    ) -> Result<PromotedAttendee> {
        let (id, version, catalog_version, attendee_id) = (
            id.to_owned(),
            expected_version.to_owned(),
            expected_catalog_version.to_owned(),
            attendee_id.to_owned(),
        );
        self.call(move |engine| {
            engine.promote_meeting_attendee(&id, &version, &catalog_version, &attendee_id, name)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{reader::LibraryReader, MeetingStatus};
    use serde_json::json;

    fn engine() -> (tempfile::TempDir, Engine) {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().canonicalize().unwrap().join("library");
        let engine = Engine::open(root).unwrap();
        (temporary, engine)
    }
    fn person(name: &str) -> Person {
        Person {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.into(),
        }
    }
    fn attendee(name: Option<&str>) -> MeetingAttendee {
        MeetingAttendee {
            id: uuid::Uuid::new_v4().to_string(),
            person_id: None,
            name: name.map(str::to_owned),
        }
    }
    fn replace(engine: &mut Engine, id: &str, attendees: Vec<MeetingAttendee>) -> Meeting {
        let version = agent_control::meeting_edit_version(&engine.meetings[id].meeting).unwrap();
        engine
            .replace_meeting_attendees(id, &version, attendees)
            .unwrap()
    }

    #[test]
    fn catalog_names_and_groups_do_not_rewrite_meeting_snapshots() {
        let (_temporary, mut engine) = engine();
        let meeting = engine
            .create_meeting("Synthetic meeting".into(), "Notes".into())
            .unwrap();
        let first = person("Same name");
        let second = person("Same name");
        let group = PersonGroup {
            id: uuid::Uuid::new_v4().to_string(),
            name: "Group".into(),
            person_ids: vec![first.id.clone(), second.id.clone()],
        };
        let empty = engine.people_catalog().unwrap();
        let catalog = engine
            .replace_people_catalog(
                &empty.edit_version,
                vec![first.clone(), second.clone()],
                vec![group],
            )
            .unwrap();
        let selected = MeetingAttendee {
            id: uuid::Uuid::new_v4().to_string(),
            person_id: Some(first.id.clone()),
            name: Some(first.name.clone()),
        };
        let meeting = replace(
            &mut engine,
            &meeting.id,
            vec![selected.clone(), attendee(None), attendee(None)],
        );
        let original_version = agent_control::meeting_edit_version(&meeting).unwrap();
        let mut renamed = first.clone();
        renamed.name = "Changed catalog name".into();
        let changed = engine
            .replace_people_catalog(&catalog.edit_version, vec![renamed, second], Vec::new())
            .unwrap();
        assert_eq!(
            engine.meetings[&meeting.id].meeting.attendees,
            meeting.attendees
        );
        assert_eq!(
            agent_control::meeting_edit_version(&engine.meetings[&meeting.id].meeting).unwrap(),
            original_version
        );
        let deleted = engine
            .replace_people_catalog(&changed.edit_version, Vec::new(), Vec::new())
            .unwrap();
        assert_eq!(deleted.people.len(), 0);
        let mut reordered = meeting.attendees.clone();
        reordered.reverse();
        let updated = replace(&mut engine, &meeting.id, reordered);
        assert!(updated.attendees.contains(&selected));
        let new_link = MeetingAttendee {
            id: uuid::Uuid::new_v4().to_string(),
            ..selected
        };
        assert!(engine
            .replace_meeting_attendees(
                &meeting.id,
                &agent_control::meeting_edit_version(&updated).unwrap(),
                vec![new_link]
            )
            .is_err());
        assert!(matches!(
            engine.replace_people_catalog(&catalog.edit_version, Vec::new(), Vec::new()),
            Err(CoreError::EditConflict)
        ));
    }

    #[test]
    fn roster_edits_are_source_preserving_cas_projections_in_every_meeting_state() {
        let (_temporary, mut engine) = engine();
        let meeting = engine
            .create_meeting("Synthetic meeting".into(), "Preserved notes".into())
            .unwrap();
        let path = engine
            .store
            .root
            .join("meetings")
            .join(&meeting.id)
            .join("meeting.json");
        let original = std::fs::read(&path).unwrap();
        let first = attendee(None);
        let meeting = replace(&mut engine, &meeting.id, vec![first.clone()]);
        assert_eq!(std::fs::read(&path).unwrap(), original);
        let before = agent_control::meeting_edit_version(&meeting).unwrap();
        let mut renamed = first.clone();
        renamed.name = Some("Local name".into());
        let changed = replace(&mut engine, &meeting.id, vec![renamed.clone()]);
        assert!(matches!(
            engine.replace_meeting_attendees(&meeting.id, &before, vec![first]),
            Err(CoreError::EditConflict)
        ));
        assert_eq!(changed.attendees[0].id, renamed.id);
        for status in [
            MeetingStatus::Recording,
            MeetingStatus::Paused,
            MeetingStatus::Completed,
            MeetingStatus::Interrupted,
        ] {
            engine.meetings.get_mut(&meeting.id).unwrap().meeting.status = status;
            replace(&mut engine, &meeting.id, vec![renamed.clone()]);
        }
        assert_eq!(std::fs::read(&path).unwrap(), original);
        let root = engine.store.root.clone();
        drop(engine);
        let engine = Engine::open(root.clone()).unwrap();
        assert_eq!(
            engine.meetings[&meeting.id].meeting.attendees,
            vec![renamed.clone()]
        );
        let before = std::fs::read(root.join("library.json")).unwrap();
        let reader = LibraryReader::open(&root).unwrap();
        let read = reader
            .query("meeting_get", json!({"meetingId":meeting.id}))
            .unwrap();
        assert_eq!(read["meeting"]["attendees"], json!([renamed]));
        assert_eq!(
            read["editVersion"],
            agent_control::meeting_edit_version(&engine.meetings[&meeting.id].meeting).unwrap()
        );
        assert_eq!(std::fs::read(root.join("library.json")).unwrap(), before);
        assert_eq!(std::fs::read(path).unwrap(), original);
    }

    #[test]
    fn promotion_checks_both_versions_and_commits_catalog_and_roster_together() {
        let (_temporary, mut engine) = engine();
        let meeting = engine
            .create_meeting("Synthetic meeting".into(), String::new())
            .unwrap();
        let local = attendee(None);
        let meeting = replace(&mut engine, &meeting.id, vec![local.clone()]);
        let version = agent_control::meeting_edit_version(&meeting).unwrap();
        let catalog = engine.people_catalog().unwrap();
        let current = engine
            .replace_people_catalog(&catalog.edit_version, vec![person("Existing")], Vec::new())
            .unwrap();
        assert!(matches!(
            engine.promote_meeting_attendee(
                &meeting.id,
                &version,
                &catalog.edit_version,
                &local.id,
                "Promoted".into()
            ),
            Err(CoreError::EditConflict)
        ));
        assert_eq!(engine.people_catalog().unwrap().people.len(), 1);
        assert_eq!(
            engine.meetings[&meeting.id].meeting.attendees,
            meeting.attendees
        );
        let promoted = engine
            .promote_meeting_attendee(
                &meeting.id,
                &version,
                &current.edit_version,
                &local.id,
                "Promoted".into(),
            )
            .unwrap();
        assert_eq!(promoted.meeting.attendees[0].id, local.id);
        assert_eq!(
            promoted.meeting.attendees[0].person_id.as_ref(),
            Some(&promoted.person.id)
        );
        assert_eq!(promoted.catalog.people.last(), Some(&promoted.person));
        let library: serde_json::Value =
            serde_json::from_slice(&std::fs::read(engine.store.root.join("library.json")).unwrap())
                .unwrap();
        assert_eq!(library["people"]["people"][1]["id"], promoted.person.id);
        assert_eq!(
            library["meetingAttendees"][&meeting.id][0]["personId"],
            promoted.person.id
        );
        let current_version = agent_control::meeting_edit_version(&promoted.meeting).unwrap();
        assert!(engine
            .promote_meeting_attendee(
                &meeting.id,
                &current_version,
                &promoted.catalog.edit_version,
                &local.id,
                "Duplicate".into()
            )
            .is_err());
    }

    #[test]
    fn duplicate_links_and_invalid_catalogs_are_rejected_without_writes() {
        let (_temporary, mut engine) = engine();
        let meeting = engine
            .create_meeting("Synthetic meeting".into(), String::new())
            .unwrap();
        let version = agent_control::meeting_edit_version(&meeting).unwrap();
        let one = person("Person");
        let catalog = engine.people_catalog().unwrap();
        assert!(engine
            .replace_people_catalog(
                &catalog.edit_version,
                vec![one.clone(), one.clone()],
                Vec::new()
            )
            .is_err());
        let group = PersonGroup {
            id: uuid::Uuid::new_v4().to_string(),
            name: "Group".into(),
            person_ids: vec![one.id.clone()],
        };
        assert!(engine
            .replace_people_catalog(&catalog.edit_version, Vec::new(), vec![group])
            .is_err());
        let anonymous = attendee(None);
        assert!(engine
            .replace_meeting_attendees(&meeting.id, &version, vec![anonymous.clone(), anonymous])
            .is_err());
        let linked = MeetingAttendee {
            id: uuid::Uuid::new_v4().to_string(),
            person_id: Some(one.id.clone()),
            name: Some(one.name.clone()),
        };
        let mut duplicate = linked.clone();
        duplicate.id = uuid::Uuid::new_v4().to_string();
        assert!(validate_attendees(&[linked, duplicate]).is_err());
        assert!(!engine.store.root.join("library.json").exists());
        engine.trash_meetings(&[meeting.id.clone()]).unwrap();
        assert!(matches!(
            engine.replace_meeting_attendees(&meeting.id, &version, Vec::new()),
            Err(CoreError::MeetingNotFound)
        ));
    }

    #[test]
    fn agent_mutations_validate_and_apply_catalog_roster_and_promotion_cas() {
        let (_temporary, mut engine) = engine();
        let meeting = engine
            .create_meeting("Synthetic meeting".into(), String::new())
            .unwrap();
        let catalog = engine.people_catalog().unwrap();
        let payload = json!({"expectedVersion":catalog.edit_version,"people":[],"groups":[]});
        let request = agent_control::Mutation::parse("people_update", payload).unwrap();
        assert_eq!(
            agent_control::apply(&mut engine, request).unwrap()["applied"],
            true
        );
        let attendee = attendee(None);
        let payload = json!({"meetingId":meeting.id,"expectedVersion":agent_control::meeting_edit_version(&meeting).unwrap(),"attendees":[attendee]});
        let request =
            agent_control::Mutation::parse("meeting_attendees_set", payload.clone()).unwrap();
        let applied = agent_control::apply(&mut engine, request).unwrap();
        let old = agent_control::Mutation::parse("meeting_attendees_set", payload).unwrap();
        assert!(matches!(
            agent_control::apply(&mut engine, old),
            Err(CoreError::EditConflict)
        ));
        let request=agent_control::Mutation::parse("meeting_attendee_promote",json!({"meetingId":meeting.id,"expectedVersion":applied["editVersion"],"expectedCatalogVersion":catalog.edit_version,"attendeeId":attendee.id,"name":"Saved reference"})).unwrap();
        let promoted = agent_control::apply(&mut engine, request).unwrap();
        assert_eq!(
            promoted["personId"],
            engine.people_catalog().unwrap().people[0].id
        );
        assert!(agent_control::Mutation::parse("meeting_attendees_set",json!({"meetingId":meeting.id,"expectedVersion":applied["editVersion"],"attendees":[{"id":uuid::Uuid::new_v4().to_string(),"speakerId":"inferred"}]})).is_err());
        assert!(agent_control::Mutation::parse(
            "people_update",
            json!({"expectedVersion":"not-a-version","people":[],"groups":[]})
        )
        .is_err());
    }

    #[test]
    fn catalog_reader_pages_both_entry_types_and_rejects_changed_or_malformed_sources() {
        let (_temporary, mut engine) = engine();
        let person = person("Person");
        let group = PersonGroup {
            id: uuid::Uuid::new_v4().to_string(),
            name: "Group".into(),
            person_ids: vec![person.id.clone()],
        };
        let catalog = engine.people_catalog().unwrap();
        engine
            .replace_people_catalog(&catalog.edit_version, vec![person], vec![group])
            .unwrap();
        let reader = LibraryReader::open(&engine.store.root).unwrap();
        let first = reader.query("people_catalog", json!({"limit":1})).unwrap();
        assert_eq!(first["entries"][0]["kind"], "person");
        assert_eq!(first["pagination"]["nextOffset"], 1);
        let second = reader
            .query(
                "people_catalog",
                json!({"offset":1,"limit":1,"expectedSourceVersion":first["sourceVersion"]}),
            )
            .unwrap();
        assert_eq!(second["entries"][0]["kind"], "group");
        assert!(reader
            .query(
                "people_catalog",
                json!({"meetingId":uuid::Uuid::new_v4().to_string()})
            )
            .is_err());
        let path = engine.store.root.join("library.json");
        let mut library: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        library["people"]["people"][0]["name"] = json!("Changed");
        std::fs::write(&path, serde_json::to_vec(&library).unwrap()).unwrap();
        assert!(matches!(
            reader.query(
                "people_catalog",
                json!({"offset":1,"expectedSourceVersion":first["sourceVersion"]})
            ),
            Err(CoreError::Reader(
                crate::reader::ReadError::SourceVersionMismatch
            ))
        ));
        library["people"]["people"][0]["unexpected"] = json!(true);
        std::fs::write(&path, serde_json::to_vec(&library).unwrap()).unwrap();
        assert!(reader.query("people_catalog", json!({})).is_err());
    }
}
