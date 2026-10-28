use crate::QueryError;
use jiff::{civil::Date, tz::TimeZone, Timestamp};
use serde_json::{json, Value};

fn invalid_date() -> QueryError {
    QueryError {
        code: "invalid_input",
        message: "날짜는 YYYY-MM-DD와 timezone(IANA)을 함께 지정하세요. on과 from/to는 함께 사용할 수 없으며 from 포함, to 미포함입니다.",
    }
}

pub(crate) fn time_zone(value: Option<&Value>) -> Result<TimeZone, QueryError> {
    match value {
        None => Ok(TimeZone::UTC),
        Some(Value::String(name)) => {
            if name.len() > 255
                || name.split('/').any(str::is_empty)
                || !name.bytes().all(|byte| {
                    byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'_' | b'-' | b'+')
                })
            {
                return Err(invalid_date());
            }
            TimeZone::get(name).map_err(|_| invalid_date())
        }
        Some(_) => Err(invalid_date()),
    }
}

fn date(value: &str) -> Result<Date, QueryError> {
    let bytes = value.as_bytes();
    if bytes.len() != 10
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes
            .iter()
            .enumerate()
            .any(|(index, value)| !matches!(index, 4 | 7) && !value.is_ascii_digit())
    {
        return Err(invalid_date());
    }
    value.parse().map_err(|_| invalid_date())
}

fn midnight(date: Date, zone: &TimeZone) -> Result<u64, QueryError> {
    let zoned = zone
        .to_ambiguous_zoned(date.at(0, 0, 0, 0))
        .unambiguous()
        .map_err(|_| invalid_date())?;
    u64::try_from(zoned.timestamp().as_millisecond()).map_err(|_| invalid_date())
}

pub(crate) fn normalize(method: &str, params: Value) -> Result<Value, QueryError> {
    if !matches!(method, "meetings_list" | "meetings_search") {
        return Ok(params);
    }
    let mut params = params.as_object().ok_or_else(QueryError::invalid)?.clone();
    let explicit_zone = params.remove("timezone");
    let zone = time_zone(explicit_zone.as_ref())?;
    if let Some(on) = params.remove("on") {
        if params.contains_key("from") || params.contains_key("to") || explicit_zone.is_none() {
            return Err(invalid_date());
        }
        let day = date(on.as_str().ok_or_else(invalid_date)?)?;
        params.insert("from".into(), json!(midnight(day, &zone)?));
        params.insert(
            "to".into(),
            json!(midnight(
                day.tomorrow().map_err(|_| invalid_date())?,
                &zone
            )?),
        );
    }
    for field in ["from", "to"] {
        if let Some(Value::String(value)) = params.get(field) {
            if explicit_zone.is_none() {
                return Err(invalid_date());
            }
            let boundary = midnight(date(value)?, &zone)?;
            params.insert(field.into(), json!(boundary));
        }
    }
    Ok(Value::Object(params))
}

pub(crate) fn timestamp(value: &Value, zone: &TimeZone) -> String {
    value
        .as_u64()
        .and_then(|value| i64::try_from(value).ok())
        .and_then(|value| Timestamp::from_millisecond(value).ok())
        .map(|value| {
            value
                .to_zoned(zone.clone())
                .strftime("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_else(|| "-".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_calendar_days_use_local_boundaries_across_daylight_saving() {
        for (day, hours) in [("2026-03-08", 23), ("2026-11-01", 25)] {
            let params = normalize(
                "meetings_list",
                json!({"on":day,"timezone":"America/New_York"}),
            )
            .unwrap();
            assert_eq!(
                params["to"].as_u64().unwrap() - params["from"].as_u64().unwrap(),
                hours * 60 * 60 * 1000
            );
            assert!(params.get("on").is_none());
            assert!(params.get("timezone").is_none());
        }
    }

    #[test]
    fn date_filters_require_an_explicit_zone_and_reject_ambiguous_boundaries() {
        for params in [
            json!({"on":"2026-10-28"}),
            json!({"from":"2026-10-28"}),
            json!({"on":"2026-10-28","timezone":"Invalid/Zone"}),
            json!({"on":"2026-10-28","timezone":42}),
            json!({"on":"2026-10-28","timezone":"/etc/localtime"}),
            json!({"on":"2026-10-28","timezone":"../../etc/localtime"}),
            json!({"on":"2026-10-28","timezone":"Asia//Seoul"}),
            json!({"on":"2026-02-30","timezone":"UTC"}),
            json!({"on":"2026-1-2","timezone":"UTC"}),
            json!({"on":"2026-10-28","from":0,"timezone":"UTC"}),
            json!({"on":"2018-11-04","timezone":"America/Sao_Paulo"}),
            json!({"on":"2011-12-30","timezone":"Pacific/Apia"}),
        ] {
            assert_eq!(
                normalize("meetings_list", params).unwrap_err().code,
                "invalid_input"
            );
        }
    }

    #[test]
    fn epoch_filters_are_unchanged_and_text_dates_are_timezone_bound() {
        assert_eq!(
            normalize("meetings_list", json!({"from":1000,"to":2000})).unwrap(),
            json!({"from":1000,"to":2000})
        );
        let params = normalize(
            "meetings_search",
            json!({"query":"term","from":"2026-10-28","to":"2026-10-29","timezone":"Asia/Seoul"}),
        )
        .unwrap();
        let zone = TimeZone::get("Asia/Seoul").unwrap();
        assert_eq!(timestamp(&params["from"], &zone), "2026-10-28 00:00");
        assert_eq!(timestamp(&params["to"], &zone), "2026-10-29 00:00");
        assert_eq!(params["query"], "term");
    }
}
