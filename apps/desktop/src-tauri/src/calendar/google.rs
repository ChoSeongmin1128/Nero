use super::oauth::{Config, Pending, SCOPES};
use super::schedule::Occurrence;
use super::tokens::StoredToken;
use jiff::Timestamp;
use reqwest::{Client, StatusCode, Url};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

const API: &str = "https://www.googleapis.com/calendar/v3/";
const MAX_PAGES: usize = 100;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CalendarChoice {
    pub id: String,
    pub name: String,
    pub primary: bool,
}

#[derive(Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_in: u64,
    pub scope: Option<String>,
    pub token_type: String,
}

#[derive(Debug)]
pub enum Failure {
    Reconnect,
    Gone,
    Other(String),
}
impl Failure {
    pub fn message(&self) -> String {
        match self {
            Self::Reconnect => {
                "Google 연결이 만료되었거나 권한이 해제되었습니다. 다시 연결해 주세요.".into()
            }
            Self::Gone => "캘린더 변경 정보를 다시 확인해야 합니다. 새로고침해 주세요.".into(),
            Self::Other(message) => message.clone(),
        }
    }
}
fn invalid() -> Failure {
    Failure::Other("Google 캘린더 응답을 확인하지 못했습니다. 다시 새로고침해 주세요.".into())
}

async fn read_json(mut response: reqwest::Response, maximum: usize) -> Result<Value, Failure> {
    if response
        .content_length()
        .is_some_and(|length| length > maximum as u64)
    {
        return Err(invalid());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| invalid())? {
        if bytes.len().saturating_add(chunk.len()) > maximum {
            return Err(invalid());
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| invalid())
}

async fn response_json(response: reqwest::Response, maximum: usize) -> Result<Value, Failure> {
    match response.status() {
        StatusCode::UNAUTHORIZED => return Err(Failure::Reconnect),
        StatusCode::GONE => return Err(Failure::Gone),
        value if !value.is_success() => {
            return Err(Failure::Other(
                "Google 캘린더에 연결하지 못했습니다. 잠시 후 다시 시도해 주세요.".into(),
            ))
        }
        _ => (),
    }
    read_json(response, maximum).await
}

async fn token_request(
    client: &Client,
    config: &Config,
    fields: &[(&str, &str)],
) -> Result<TokenResponse, Failure> {
    let mut form = Url::parse("https://oauth2.googleapis.com/token").unwrap();
    {
        let mut pairs = form.query_pairs_mut();
        pairs.append_pair("client_id", &config.client_id);
        if let Some(secret) = &config.client_secret {
            pairs.append_pair("client_secret", secret);
        }
        pairs.extend_pairs(fields.iter().copied());
    }
    let response = client
        .post("https://oauth2.googleapis.com/token")
        .header(
            reqwest::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .body(form.query().unwrap_or_default().to_owned())
        .send()
        .await
        .map_err(|_| {
            Failure::Other(
                "Google 연결을 확인하지 못했습니다. 네트워크 상태를 확인해 주세요.".into(),
            )
        })?;
    if response.status() == StatusCode::BAD_REQUEST {
        let body = read_json(response, 64 * 1024).await?;
        return Err(if body["error"] == "invalid_grant" {
            Failure::Reconnect
        } else {
            Failure::Other(
                "Google 연결 정보를 확인하지 못했습니다. 잠시 후 다시 시도해 주세요.".into(),
            )
        });
    }
    let token: TokenResponse =
        serde_json::from_value(response_json(response, 64 * 1024).await?).map_err(|_| invalid())?;
    if token.access_token.is_empty()
        || token.expires_in == 0
        || !token.token_type.eq_ignore_ascii_case("bearer")
    {
        return Err(invalid());
    }
    if token.scope.as_ref().is_some_and(|scope| {
        SCOPES
            .split(' ')
            .any(|required| !scope.split(' ').any(|granted| granted == required))
    }) {
        return Err(Failure::Reconnect);
    }
    Ok(token)
}

pub async fn exchange(
    client: &Client,
    config: &Config,
    pending: &Pending,
    code: &str,
) -> Result<TokenResponse, Failure> {
    token_request(
        client,
        config,
        &[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("code_verifier", &pending.verifier),
            ("redirect_uri", &pending.redirect_uri),
        ],
    )
    .await
}
pub async fn refresh(
    client: &Client,
    config: &Config,
    token: &StoredToken,
) -> Result<TokenResponse, Failure> {
    if token.client_id != config.client_id {
        return Err(Failure::Reconnect);
    }
    token_request(
        client,
        config,
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", &token.refresh_token),
        ],
    )
    .await
}

async fn get(client: &Client, token: &str, url: Url) -> Result<Value, Failure> {
    let response = client
        .get(url)
        .bearer_auth(token)
        .send()
        .await
        .map_err(|_| {
            Failure::Other("일정을 새로고침하지 못했습니다. 네트워크 상태를 확인해 주세요.".into())
        })?;
    response_json(response, 4 * 1024 * 1024).await
}
fn next_page(page: &Value, seen: &mut BTreeSet<String>) -> Result<Option<String>, Failure> {
    let Some(token) = page.get("nextPageToken") else {
        return Ok(None);
    };
    let token = token
        .as_str()
        .filter(|token| !token.is_empty() && token.len() <= 8192)
        .ok_or_else(invalid)?;
    if !seen.insert(token.into()) {
        return Err(invalid());
    }
    Ok(Some(token.into()))
}

fn items(page: &Value) -> Result<&[Value], Failure> {
    if !page.is_object() {
        return Err(invalid());
    }
    match page.get("items") {
        None => Ok(&[]),
        Some(Value::Array(items)) => Ok(items),
        _ => Err(invalid()),
    }
}

pub async fn calendars(client: &Client, token: &str) -> Result<Vec<CalendarChoice>, Failure> {
    let mut result = Vec::new();
    let mut page_token: Option<String> = None;
    let mut seen = BTreeSet::new();
    for _ in 0..MAX_PAGES {
        let mut url = Url::parse(&format!("{API}users/me/calendarList")).unwrap();
        url.query_pairs_mut().extend_pairs([
            ("maxResults", "250"),
            ("minAccessRole", "reader"),
            ("fields", "items(id,summary,primary,deleted),nextPageToken"),
        ]);
        if let Some(token) = &page_token {
            url.query_pairs_mut().append_pair("pageToken", token);
        }
        let page = get(client, token, url).await?;
        for calendar in items(&page)? {
            if calendar["deleted"] == true {
                continue;
            }
            let id = calendar["id"]
                .as_str()
                .filter(|value| !value.is_empty() && value.len() <= 1024)
                .ok_or_else(invalid)?;
            result.push(CalendarChoice {
                id: id.into(),
                name: calendar["summary"]
                    .as_str()
                    .unwrap_or("제목 없는 캘린더")
                    .into(),
                primary: calendar["primary"] == true,
            });
        }
        page_token = next_page(&page, &mut seen)?;
        if page_token.is_none() {
            return Ok(result);
        }
    }
    Err(Failure::Other(
        "캘린더 목록이 조회 한도를 넘었습니다.".into(),
    ))
}

pub async fn events(
    client: &Client,
    token: &str,
    account: &str,
    selected: &[CalendarChoice],
    now: u64,
) -> Result<Vec<Occurrence>, Failure> {
    let from = Timestamp::from_millisecond(i64::try_from(now).map_err(|_| invalid())?)
        .map_err(|_| invalid())?
        .to_string();
    let to = Timestamp::from_millisecond(
        i64::try_from(now.saturating_add(24 * 60 * 60 * 1000)).map_err(|_| invalid())?,
    )
    .map_err(|_| invalid())?
    .to_string();
    let mut result = BTreeMap::<String, (u64, Occurrence)>::new();
    for calendar in selected {
        let mut page_token: Option<String> = None;
        let mut seen = BTreeSet::new();
        let mut complete = false;
        for _ in 0..MAX_PAGES {
            let mut url = Url::parse(API).unwrap();
            url.path_segments_mut()
                .map_err(|_| invalid())?
                .pop_if_empty()
                .extend(["calendars", calendar.id.as_str(), "events"]);
            url.query_pairs_mut().extend_pairs([
                ("singleEvents", "true"), ("showDeleted", "true"), ("maxResults", "2500"),
                ("timeMin", from.as_str()), ("timeMax", to.as_str()),
                ("fields", "items(id,status,summary,start,end,eventType,recurringEventId,originalStartTime,iCalUID,updated,attendees(self,responseStatus),conferenceData(entryPoints(entryPointType,uri))),nextPageToken"),
            ]);
            if let Some(token) = &page_token {
                url.query_pairs_mut().append_pair("pageToken", token);
            }
            let page = get(client, token, url).await?;
            for raw in items(&page)? {
                if let Some((updated, event)) = normalize(account, calendar, raw)? {
                    if result
                        .get(&event.key)
                        .is_none_or(|(previous, _)| *previous < updated)
                    {
                        result.insert(event.key.clone(), (updated, event));
                    }
                }
            }
            page_token = next_page(&page, &mut seen)?;
            if page_token.is_none() {
                complete = true;
                break;
            }
        }
        if !complete {
            return Err(Failure::Other(
                "일정이 조회 한도를 넘었습니다. 선택한 캘린더를 줄여 주세요.".into(),
            ));
        }
    }
    Ok(result.into_values().map(|(_, event)| event).collect())
}

fn event_time(value: &Value) -> Result<u64, Failure> {
    let text = value["dateTime"].as_str().ok_or_else(invalid)?;
    let time = match text.parse::<Timestamp>() {
        Ok(time) => time,
        Err(_) => {
            let zone = value["timeZone"].as_str().ok_or_else(invalid)?;
            let zone = jiff::tz::TimeZone::get(zone).map_err(|_| invalid())?;
            let date = text
                .parse::<jiff::civil::DateTime>()
                .map_err(|_| invalid())?;
            zone.to_ambiguous_zoned(date)
                .unambiguous()
                .map_err(|_| invalid())?
                .timestamp()
        }
    };
    u64::try_from(time.as_millisecond()).map_err(|_| invalid())
}
fn normalize(
    account: &str,
    calendar: &CalendarChoice,
    raw: &Value,
) -> Result<Option<(u64, Occurrence)>, Failure> {
    if raw["status"] == "cancelled"
        || raw["start"]["date"].is_string()
        || raw["end"]["date"].is_string()
        || raw["attendees"].as_array().is_some_and(|attendees| {
            attendees.iter().any(|attendee| {
                attendee["self"] == true && attendee["responseStatus"] == "declined"
            })
        })
        || !matches!(
            raw["eventType"].as_str(),
            None | Some("default" | "fromGmail")
        )
    {
        return Ok(None);
    }
    let id = raw["id"]
        .as_str()
        .filter(|id| !id.is_empty())
        .ok_or_else(invalid)?;
    let start = event_time(&raw["start"])?;
    let end = event_time(&raw["end"])?;
    if end <= start {
        return Err(invalid());
    }
    let original = if raw["recurringEventId"].is_string() {
        event_time(&raw["originalStartTime"])?.to_string()
    } else {
        String::new()
    };
    let identity = if let Some(uid) = raw["iCalUID"].as_str().filter(|uid| !uid.is_empty()) {
        serde_json::to_vec(&(account, "ical", uid, &original))
    } else {
        serde_json::to_vec(&(account, &calendar.id, id, &original))
    }
    .map_err(|_| invalid())?;
    let key = format!("google:{:x}", Sha256::digest(identity));
    let mut links = BTreeSet::new();
    if let Some(entries) = raw["conferenceData"]["entryPoints"].as_array() {
        for entry in entries
            .iter()
            .filter(|entry| entry["entryPointType"] == "video")
        {
            if let Some(uri) = entry["uri"].as_str() {
                if let Ok(url) = Url::parse(uri) {
                    if url.scheme() == "https"
                        && url.host_str().is_some()
                        && url.username().is_empty()
                        && url.password().is_none()
                    {
                        links.insert(url.to_string());
                    }
                }
            }
        }
    }
    let updated = raw["updated"]
        .as_str()
        .and_then(|text| text.parse::<Timestamp>().ok())
        .and_then(|value| u64::try_from(value.as_millisecond()).ok())
        .unwrap_or(0);
    Ok(Some((
        updated,
        Occurrence {
            key,
            title: raw["summary"]
                .as_str()
                .filter(|title| !title.trim().is_empty())
                .unwrap_or("제목 없는 일정")
                .into(),
            start_at: start,
            end_at: end,
            calendar_name: calendar.name.clone(),
            join_urls: links.into_iter().collect(),
        },
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn calendar() -> CalendarChoice {
        CalendarChoice {
            id: "synthetic-calendar".into(),
            name: "Synthetic".into(),
            primary: true,
        }
    }
    fn event() -> Value {
        json!({"id":"event-one","status":"confirmed","summary":"Synthetic meeting","start":{"dateTime":"2026-11-01T01:30:00-04:00"},"end":{"dateTime":"2026-11-01T01:45:00-04:00"},"iCalUID":"synthetic-uid"})
    }
    #[test]
    fn normalization_handles_exclusions_recurrence_dst_and_safe_join_urls() {
        let base = event();
        let normalized = normalize("account", &calendar(), &base).unwrap().unwrap().1;
        assert_eq!(
            normalized.start_at,
            "2026-11-01T05:30:00Z"
                .parse::<Timestamp>()
                .unwrap()
                .as_millisecond() as u64
        );
        for patch in [
            json!({"status":"cancelled"}),
            json!({"start":{"date":"2026-11-01"}}),
            json!({"attendees":[{"self":true,"responseStatus":"declined"}]}),
        ] {
            let mut excluded = base.clone();
            excluded
                .as_object_mut()
                .unwrap()
                .extend(patch.as_object().unwrap().clone());
            assert!(normalize("account", &calendar(), &excluded)
                .unwrap()
                .is_none());
        }
        let mut repeated = base.clone();
        repeated["recurringEventId"] = json!("series");
        repeated["originalStartTime"] = base["start"].clone();
        let original_key = normalize("account", &calendar(), &repeated)
            .unwrap()
            .unwrap()
            .1
            .key;
        repeated["start"]["dateTime"] = json!("2026-11-01T01:30:00-05:00");
        repeated["end"]["dateTime"] = json!("2026-11-01T01:45:00-05:00");
        assert_eq!(
            normalize("account", &calendar(), &repeated)
                .unwrap()
                .unwrap()
                .1
                .key,
            original_key
        );
        repeated["conferenceData"]["entryPoints"] = json!([{"entryPointType":"video","uri":"javascript:alert(1)"},{"entryPointType":"video","uri":"https://meet.google.com/synthetic"},{"entryPointType":"phone","uri":"tel:+123"}]);
        assert_eq!(
            normalize("account", &calendar(), &repeated)
                .unwrap()
                .unwrap()
                .1
                .join_urls,
            vec!["https://meet.google.com/synthetic"]
        );
        let mut mirrored = calendar();
        mirrored.id = "other-calendar".into();
        assert_eq!(
            normalize("account", &mirrored, &base)
                .unwrap()
                .unwrap()
                .1
                .key,
            normalized.key
        );
    }
    #[test]
    fn repeated_page_tokens_and_ambiguous_local_times_are_rejected() {
        assert!(items(&json!({})).unwrap().is_empty());
        assert!(items(&json!({"items":null})).is_err());
        let mut seen = BTreeSet::new();
        assert_eq!(
            next_page(&json!({"nextPageToken":"one"}), &mut seen).unwrap(),
            Some("one".into())
        );
        assert!(next_page(&json!({"nextPageToken":"one"}), &mut seen).is_err());
        assert!(event_time(
            &json!({"dateTime":"2026-11-01T01:30:00","timeZone":"America/New_York"})
        )
        .is_err());
    }
}
