use super::{google, CalendarService};
use google::{CalendarChoice, Failure};
use jiff::Timestamp;
use nero_core::people::{GoogleAttendeeKind, GoogleAttendeeOrigin, GoogleAttendeeSource};
use nero_core::CalendarReference;
use reqwest::{Client, Url};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::Ordering;

const MAX_PAGES: usize = 100;
const MAX_SOURCES: usize = 10_000;
const CALENDAR_FIELDS: &str = "items(id,status,summary,start,end,eventType,recurringEventId,originalStartTime,iCalUID,updated,attendeesOmitted,attendees(id,email,displayName,resource,responseStatus,self),conferenceData(entryPoints(entryPointType,uri))),nextPageToken";

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum MeetAttendeeStatus {
    NotRequested,
    PermissionRequired,
    NoMeetLink,
    NoConference,
    AmbiguousConference,
    Available,
    Unavailable,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GoogleAttendeeFetch {
    pub sources: Vec<GoogleAttendeeSource>,
    pub calendar_complete: bool,
    pub meet_status: MeetAttendeeStatus,
    pub warning: Option<String>,
}

fn invalid() -> Failure {
    Failure::Other("Google 참석자 응답을 확인하지 못했습니다. 다시 불러와 주세요.".into())
}

fn timestamp(value: u64) -> Result<String, Failure> {
    Timestamp::from_millisecond(i64::try_from(value).map_err(|_| invalid())?)
        .map(|value| value.to_string())
        .map_err(|_| invalid())
}

fn millis(value: &Value) -> Result<u64, Failure> {
    let timestamp = value
        .as_str()
        .ok_or_else(invalid)?
        .parse::<Timestamp>()
        .map_err(|_| invalid())?;
    u64::try_from(timestamp.as_millisecond()).map_err(|_| invalid())
}

fn display_name(value: &Value) -> Option<String> {
    value
        .as_str()
        .map(str::trim)
        .filter(|name| {
            !name.is_empty() && name.chars().count() <= 120 && !name.chars().any(char::is_control)
        })
        .map(str::to_owned)
}

fn email(value: &Value) -> Option<String> {
    value
        .as_str()
        .map(str::trim)
        .filter(|email| {
            email.len() <= 320
                && !email
                    .chars()
                    .any(|value| value.is_control() || value.is_whitespace())
                && email.split_once('@').is_some_and(|(local, domain)| {
                    !local.is_empty() && !domain.is_empty() && !domain.contains('@')
                })
        })
        .map(str::to_ascii_lowercase)
}

fn calendar_sources(
    account: &str,
    occurrence_key: &str,
    raw: &Value,
) -> Result<Vec<GoogleAttendeeSource>, Failure> {
    let attendees = match raw.get("attendees") {
        None => return Ok(vec![]),
        Some(Value::Array(entries)) => entries,
        _ => return Err(invalid()),
    };
    if attendees.len() > MAX_SOURCES {
        return Err(invalid());
    }
    let mut result = vec![];
    for (index, attendee) in attendees.iter().enumerate() {
        if !attendee.is_object() {
            return Err(invalid());
        }
        let email = email(&attendee["email"]);
        let identity = email
            .clone()
            .or_else(|| attendee["id"].as_str().map(str::to_owned))
            .unwrap_or_else(|| {
                format!(
                    "unidentified:{index}:{:x}",
                    Sha256::digest(attendee.to_string().as_bytes())
                )
            });
        let key =
            serde_json::to_vec(&(account, occurrence_key, identity)).map_err(|_| invalid())?;
        let source = GoogleAttendeeSource {
            source_key: format!("calendar:{:x}", Sha256::digest(key)),
            origin: GoogleAttendeeOrigin::CalendarInvitation,
            kind: if attendee["resource"] == true {
                GoogleAttendeeKind::CalendarResource
            } else if email.is_some() {
                GoogleAttendeeKind::Person
            } else {
                GoogleAttendeeKind::Unknown
            },
            display_name: display_name(&attendee["displayName"]),
            email,
            google_user_id: None,
            response_status: attendee["responseStatus"].as_str().map(str::to_owned),
        };
        source.validate().map_err(|_| invalid())?;
        result.push(source);
    }
    Ok(result)
}

fn meet_code(urls: &[String]) -> Option<String> {
    let codes = urls
        .iter()
        .filter_map(|value| {
            let url = Url::parse(value).ok()?;
            if url.scheme() != "https"
                || url.host_str() != Some("meet.google.com")
                || !url.username().is_empty()
                || url.password().is_some()
            {
                return None;
            }
            let code = url.path().trim_matches('/');
            let parts = code.split('-').collect::<Vec<_>>();
            if parts.len() != 3
                || parts.iter().map(|part| part.len()).collect::<Vec<_>>() != [3, 4, 3]
                || !code
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte == b'-')
            {
                return None;
            }
            Some(code.to_owned())
        })
        .collect::<BTreeSet<_>>();
    if codes.len() == 1 {
        codes.into_iter().next()
    } else {
        None
    }
}

fn record_name(value: &str) -> bool {
    value.strip_prefix("conferenceRecords/").is_some_and(|id| {
        !id.is_empty()
            && id.len() <= 256
            && id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    })
}

fn matching_conference(
    record: &Value,
    start_at: u64,
    end_at: u64,
) -> Result<Option<String>, Failure> {
    let start = millis(&record["startTime"])?;
    let end = if record["endTime"].is_null() {
        None
    } else {
        Some(millis(&record["endTime"])?)
    };
    if end.is_some_and(|end| end < start) {
        return Err(invalid());
    }
    if start >= end_at || end.is_some_and(|end| end <= start_at) {
        return Ok(None);
    }
    record["name"]
        .as_str()
        .filter(|name| record_name(name))
        .map(|name| Some(name.to_owned()))
        .ok_or_else(invalid)
}

fn participant_source(raw: &Value, conference: &str) -> Result<GoogleAttendeeSource, Failure> {
    let resource = raw["name"].as_str().ok_or_else(invalid)?;
    let prefix = format!("{conference}/participants/");
    if !resource.strip_prefix(&prefix).is_some_and(|id| {
        !id.is_empty()
            && id.len() <= 256
            && id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    }) {
        return Err(invalid());
    }
    let variants = ["signedinUser", "anonymousUser", "phoneUser"]
        .iter()
        .filter(|key| raw.get(**key).is_some())
        .count();
    if variants > 1 {
        return Err(invalid());
    }
    let (kind, user_id, name) = if let Some(user) = raw.get("signedinUser") {
        (
            GoogleAttendeeKind::SignedInPersonOrRoom,
            Some(user["user"].as_str().ok_or_else(invalid)?.to_owned()),
            display_name(&user["displayName"]),
        )
    } else if let Some(user) = raw.get("anonymousUser") {
        (
            GoogleAttendeeKind::Anonymous,
            None,
            display_name(&user["displayName"]),
        )
    } else if let Some(user) = raw.get("phoneUser") {
        (
            GoogleAttendeeKind::Phone,
            None,
            display_name(&user["displayName"]),
        )
    } else {
        (GoogleAttendeeKind::Unknown, None, None)
    };
    let source = GoogleAttendeeSource {
        source_key: format!("meet:{resource}"),
        origin: GoogleAttendeeOrigin::MeetParticipant,
        kind,
        display_name: name,
        google_user_id: user_id,
        email: None,
        response_status: None,
    };
    source.validate().map_err(|_| invalid())?;
    Ok(source)
}

fn array<'a>(page: &'a Value, key: &str) -> Result<&'a [Value], Failure> {
    if !page.is_object() {
        return Err(invalid());
    }
    match page.get(key) {
        None => Ok(&[]),
        Some(Value::Array(entries)) => Ok(entries),
        _ => Err(invalid()),
    }
}

async fn calendar_event(
    client: &Client,
    token: &str,
    account: &str,
    calendars: &[CalendarChoice],
    reference: &CalendarReference,
) -> Result<(Value, Vec<String>), Failure> {
    let (from, to) = (timestamp(reference.start_at)?, timestamp(reference.end_at)?);
    let mut found: Option<(u64, Value, Vec<String>)> = None;
    for calendar in calendars {
        let mut next: Option<String> = None;
        let mut seen = BTreeSet::new();
        for page_index in 0..MAX_PAGES {
            let mut url = google::events_url(calendar, &from, &to, next.as_deref())?;
            let query = url
                .query_pairs()
                .filter(|(key, _)| key != "fields")
                .map(|(key, value)| (key.into_owned(), value.into_owned()))
                .collect::<Vec<_>>();
            url.set_query(None);
            url.query_pairs_mut()
                .extend_pairs(query)
                .append_pair("fields", CALENDAR_FIELDS);
            let page = google::get(client, token, url).await?;
            for raw in google::items(&page)? {
                let Ok(Some((updated, event))) = google::normalize(account, calendar, raw) else {
                    continue;
                };
                if event.key != reference.occurrence_key {
                    continue;
                }
                if event.start_at != reference.start_at || event.end_at != reference.end_at {
                    return Err(Failure::Other(
                        "연결한 일정 시간이 변경되었습니다. 일정을 다시 연결해 주세요.".into(),
                    ));
                }
                if found
                    .as_ref()
                    .is_none_or(|(previous, _, _)| *previous < updated)
                {
                    found = Some((updated, raw.clone(), event.join_urls));
                }
            }
            next = google::next_page(&page, &mut seen)?;
            if next.is_none() {
                break;
            }
            if page_index + 1 == MAX_PAGES {
                return Err(invalid());
            }
        }
    }
    found.map(|(_, raw, urls)| (raw, urls)).ok_or_else(|| {
        Failure::Other(
            "연결한 일정에 접근할 수 없거나 일정이 변경되었습니다. 일정을 다시 확인해 주세요."
                .into(),
        )
    })
}

async fn meet_sources(
    client: &Client,
    token: &str,
    code: &str,
    start_at: u64,
    end_at: u64,
) -> Result<(MeetAttendeeStatus, Vec<GoogleAttendeeSource>), Failure> {
    let mut next: Option<String> = None;
    let mut seen = BTreeSet::new();
    let mut records = BTreeMap::new();
    for page_index in 0..MAX_PAGES {
        let mut url = Url::parse("https://meet.googleapis.com/v2/conferenceRecords").unwrap();
        url.query_pairs_mut()
            .append_pair("pageSize", "100")
            .append_pair(
                "filter",
                &format!(
                    "space.meeting_code = \"{code}\" AND start_time < \"{}\"",
                    timestamp(end_at)?
                ),
            )
            .append_pair(
                "fields",
                "conferenceRecords(name,startTime,endTime),nextPageToken",
            );
        if let Some(next) = &next {
            url.query_pairs_mut().append_pair("pageToken", next);
        }
        let page = google::get(client, token, url).await?;
        for record in array(&page, "conferenceRecords")? {
            if let Some(name) = matching_conference(record, start_at, end_at)? {
                records.insert(name, ());
            }
        }
        next = google::next_page(&page, &mut seen)?;
        if next.is_none() {
            break;
        }
        if page_index + 1 == MAX_PAGES {
            return Err(invalid());
        }
    }
    if records.is_empty() {
        return Ok((MeetAttendeeStatus::NoConference, vec![]));
    }
    if records.len() != 1 {
        return Ok((MeetAttendeeStatus::AmbiguousConference, vec![]));
    }
    let conference = records.into_keys().next().unwrap();
    let mut sources = BTreeMap::new();
    next = None;
    seen.clear();
    for page_index in 0..MAX_PAGES {
        let mut url = Url::parse(&format!(
            "https://meet.googleapis.com/v2/{conference}/participants"
        ))
        .unwrap();
        url.query_pairs_mut()
            .append_pair("pageSize", "250")
            .append_pair(
                "fields",
                "participants(name,signedinUser,anonymousUser,phoneUser),nextPageToken",
            );
        if let Some(next) = &next {
            url.query_pairs_mut().append_pair("pageToken", next);
        }
        let page = google::get(client, token, url).await?;
        for participant in array(&page, "participants")? {
            let source = participant_source(participant, &conference)?;
            sources.insert(source.source_key.clone(), source);
            if sources.len() > MAX_SOURCES {
                return Err(invalid());
            }
        }
        next = google::next_page(&page, &mut seen)?;
        if next.is_none() {
            return Ok((
                MeetAttendeeStatus::Available,
                sources.into_values().collect(),
            ));
        }
        if page_index + 1 == MAX_PAGES {
            return Err(invalid());
        }
    }
    Err(invalid())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn invitation_email_resource_status_and_omissions_remain_separate_from_actual_attendance() {
        let raw = json!({"attendeesOmitted":true,"attendees":[
            {"email":"FIRST@Example.test","displayName":"Same name","responseStatus":"declined","id":"profile-100"},
            {"email":"room@example.test","displayName":"Same name","resource":true,"responseStatus":"accepted"},
            {"displayName":"Same name"}, {"displayName":"Same name"}
        ]});
        let sources = calendar_sources("synthetic", "occurrence", &raw).unwrap();
        assert_eq!(sources[0].email.as_deref(), Some("first@example.test"));
        assert!(sources.iter().all(|source| source.google_user_id.is_none()));
        assert_eq!(sources[0].response_status.as_deref(), Some("declined"));
        assert_eq!(sources[1].kind, GoogleAttendeeKind::CalendarResource);
        assert_ne!(sources[2].source_key, sources[3].source_key);
        assert!(sources
            .iter()
            .all(|source| source.origin == GoogleAttendeeOrigin::CalendarInvitation));
        let repeated = calendar_sources("synthetic", "occurrence", &raw).unwrap();
        assert_eq!(sources, repeated);
    }

    #[test]
    fn signed_in_accounts_are_not_assumed_human_and_meet_never_fabricates_email() {
        let conference = "conferenceRecords/synthetic";
        let signed = participant_source(&json!({"name":"conferenceRecords/synthetic/participants/a","signedinUser":{"user":"users/100","displayName":"Synthetic Room"}}), conference).unwrap();
        assert_eq!(signed.kind, GoogleAttendeeKind::SignedInPersonOrRoom);
        assert_eq!(signed.google_user_id.as_deref(), Some("users/100"));
        assert!(signed.email.is_none());
        let anonymous = participant_source(&json!({"name":"conferenceRecords/synthetic/participants/b","anonymousUser":{"displayName":"Synthetic Room"}}), conference).unwrap();
        let phone = participant_source(&json!({"name":"conferenceRecords/synthetic/participants/c","phoneUser":{"displayName":"1234"}}), conference).unwrap();
        assert_eq!(anonymous.kind, GoogleAttendeeKind::Anonymous);
        assert_eq!(phone.kind, GoogleAttendeeKind::Phone);
        assert!(anonymous.google_user_id.is_none() && phone.google_user_id.is_none());
        assert!(participant_source(&json!({"name":"conferenceRecords/other/participants/a","anonymousUser":{"displayName":"Same"}}), conference).is_err());
        assert!(participant_source(&json!({"name":"conferenceRecords/synthetic/participants/a","signedinUser":{"user":"users/100"},"anonymousUser":{}}), conference).is_err());
    }

    #[test]
    fn meet_links_require_an_exact_google_host_and_unambiguous_code() {
        assert_eq!(
            meet_code(&["https://meet.google.com/abc-defg-hij?authuser=0".into()]).as_deref(),
            Some("abc-defg-hij")
        );
        for link in [
            "https://meet.google.com.example.test/abc-defg-hij",
            "https://example.test/abc-defg-hij",
            "https://meet.google.com/lookup/example",
            "https://meet.google.com/abc-defg-hij/extra",
        ] {
            assert!(meet_code(&[link.into()]).is_none());
        }
        assert!(meet_code(&[
            "https://meet.google.com/abc-defg-hij".into(),
            "https://meet.google.com/klm-nopq-rst".into()
        ])
        .is_none());
    }

    #[test]
    fn reused_meet_codes_match_only_conference_intervals_overlapping_the_exact_occurrence() {
        let start = millis(&json!("2026-11-02T01:00:00Z")).unwrap();
        let end = millis(&json!("2026-11-02T02:00:00Z")).unwrap();
        for (from, to, matches) in [
            ("2026-11-02T00:00:00Z", Some("2026-11-02T01:00:00Z"), false),
            ("2026-11-02T02:00:00Z", Some("2026-11-02T03:00:00Z"), false),
            ("2026-11-02T00:58:00Z", Some("2026-11-02T01:30:00Z"), true),
            ("2026-11-02T01:01:00Z", None, true),
        ] {
            let record =
                json!({"name":"conferenceRecords/synthetic","startTime":from,"endTime":to});
            assert_eq!(
                matching_conference(&record, start, end).unwrap().is_some(),
                matches
            );
        }
        assert!(matching_conference(
            &json!({"name":"conferenceRecords/../../other","startTime":"2026-11-02T01:30:00Z"}),
            start,
            end
        )
        .is_err());
    }
}

impl CalendarService {
    pub async fn google_attendees(
        &self,
        reference: &CalendarReference,
        include_meet: bool,
    ) -> Result<GoogleAttendeeFetch, String> {
        if reference.end_at <= reference.start_at {
            return Err("연결한 일정 시간을 확인해 주세요.".into());
        }
        let generation = self.generation.load(Ordering::Acquire);
        let context = self.query_context(generation).await?;
        let (_, calendars) = self.query_calendars(&context).await?;
        let (raw, urls) = calendar_event(
            &self.client,
            &context.token,
            &context.account,
            &calendars,
            reference,
        )
        .await
        .map_err(|error| error.message())?;
        let mut sources = calendar_sources(&context.account, &reference.occurrence_key, &raw)
            .map_err(|error| error.message())?;
        let calendar_complete = raw["attendeesOmitted"] != true;
        let mut warning = (!calendar_complete)
            .then(|| "일정의 초대자 일부만 공개되어 있습니다. 저장한 참석자는 유지됩니다.".into());
        let authorized = self.inner.lock().unwrap().state.meet_authorized;
        let meet_status = if !include_meet {
            MeetAttendeeStatus::NotRequested
        } else if !authorized {
            MeetAttendeeStatus::PermissionRequired
        } else if let Some(code) = meet_code(&urls) {
            match meet_sources(
                &self.client,
                &context.token,
                &code,
                reference.start_at,
                reference.end_at,
            )
            .await
            {
                Ok((status, actual)) => {
                    sources.extend(actual);
                    status
                }
                Err(_) => {
                    warning = Some("Meet 참석자를 읽지 못했습니다. 계정의 회의 접근 권한과 Meet 연결을 확인해 주세요. 저장한 참석자는 유지됩니다.".into());
                    MeetAttendeeStatus::Unavailable
                }
            }
        } else {
            MeetAttendeeStatus::NoMeetLink
        };
        let inner = self.inner.lock().unwrap();
        self.require_browse_context(&inner, &context)?;
        Ok(GoogleAttendeeFetch {
            sources,
            calendar_complete,
            meet_status,
            warning,
        })
    }
}
