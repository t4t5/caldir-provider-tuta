use std::collections::BTreeMap;

use anyhow::{Context, Result, bail};
use caldir_core::{Event, EventTime, EventUid, Recurrence, RecurrenceId};
use chrono::{DateTime as ChronoDateTime, Duration, NaiveDate, NaiveDateTime, TimeZone, Utc};
use tutasdk::date::DateTime;
use tutasdk::entities::generated::sys::DateWrapper;
use tutasdk::entities::generated::tutanota::{
    AdvancedRepeatRule, CalendarEvent, CalendarRepeatRule,
};

const DAY_MS: u64 = 86_400_000;

pub fn to_caldir_event(source: &CalendarEvent) -> Result<Event> {
    let all_day = is_all_day(source);
    let start = from_tuta_time(source.startTime, source.startTimeZone.as_deref(), all_day)?;
    let end = from_tuta_time(source.endTime, source.endTimeZone.as_deref(), all_day)?;
    let uid = source
        .uid
        .as_deref()
        .context("Tuta calendar event has no uid")?;
    let mut event = Event::new(source.summary.clone(), start);
    event.uid = EventUid::new(uid);
    event.description = non_empty(&source.description);
    event.location = non_empty(&source.location);
    event.end = Some(end);
    event.sequence = i32::try_from(source.sequence).unwrap_or(i32::MAX);
    event.recurrence_id = source
        .recurrenceId
        .map(|value| {
            from_tuta_time(value, source.startTimeZone.as_deref(), all_day)
                .map(RecurrenceId::from_event_time)
        })
        .transpose()?;
    event.recurrence = source
        .repeatRule
        .as_ref()
        .map(|rule| recurrence_from_tuta(rule, all_day))
        .transpose()?;
    event.last_modified = None;
    Ok(event)
}

pub fn from_caldir_event(event: &Event) -> Result<CalendarEvent> {
    let (start_time, start_zone) = to_tuta_time(&event.start)?;
    let fallback_end = match event.start {
        EventTime::Date(_) => start_time.as_millis() + DAY_MS,
        _ => start_time.as_millis() + 3_600_000,
    };
    let (end_time, end_zone) = event
        .end
        .as_ref()
        .map(to_tuta_time)
        .transpose()?
        .unwrap_or((DateTime::from_millis(fallback_end), start_zone.clone()));
    let recurrence_id = event
        .recurrence_id
        .as_ref()
        .map(|id| to_tuta_time(id.as_event_time()).map(|value| value.0))
        .transpose()?;
    let repeat_rule = event
        .recurrence
        .as_ref()
        .map(|recurrence| {
            recurrence_to_tuta(
                recurrence,
                start_zone.as_deref().unwrap_or("UTC"),
                event.start.is_date(),
            )
        })
        .transpose()?;

    Ok(CalendarEvent {
        _id: None,
        _permissions: Default::default(),
        _format: 0,
        _ownerGroup: None,
        _ownerEncSessionKey: None,
        summary: event.summary.clone().unwrap_or_default(),
        description: event.description.clone().unwrap_or_default(),
        startTime: start_time,
        endTime: end_time,
        location: event.location.clone().unwrap_or_default(),
        uid: Some(event.uid.as_str().to_string()),
        hashedUid: None,
        sequence: i64::from(event.sequence),
        invitedConfidentially: None,
        recurrenceId: recurrence_id,
        _ownerKeyVersion: None,
        sender: None,
        pendingInvitation: None,
        _kdfNonce: None,
        startTimeZone: start_zone,
        endTimeZone: end_zone,
        repeatRule: repeat_rule,
        alarmInfos: Vec::new(),
        attendees: Vec::new(),
        organizer: None,
        _errors: Default::default(),
    })
}

fn non_empty(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_string())
}

fn is_all_day(event: &CalendarEvent) -> bool {
    event.startTime.as_millis() % DAY_MS == 0
        && event.endTime.as_millis() % DAY_MS == 0
        && event.endTime.as_millis() > event.startTime.as_millis()
}

fn from_tuta_time(value: DateTime, zone: Option<&str>, all_day: bool) -> Result<EventTime> {
    let utc = ChronoDateTime::<Utc>::from_timestamp_millis(
        i64::try_from(value.as_millis()).context("Tuta timestamp exceeds i64")?,
    )
    .context("Tuta timestamp is outside chrono's range")?;
    if all_day {
        return Ok(EventTime::Date(utc.date_naive()));
    }
    match zone.filter(|zone| *zone != "UTC") {
        Some(zone) => {
            let tz: chrono_tz::Tz = zone
                .parse()
                .with_context(|| format!("Unknown Tuta timezone: {zone}"))?;
            Ok(EventTime::DateTimeZoned {
                datetime: utc.with_timezone(&tz).naive_local(),
                tzid: zone.to_string(),
            })
        }
        None => Ok(EventTime::DateTimeUtc(utc)),
    }
}

fn to_tuta_time(value: &EventTime) -> Result<(DateTime, Option<String>)> {
    let (millis, zone) = match value {
        EventTime::Date(date) => (
            date.and_hms_opt(0, 0, 0)
                .context("Invalid all-day date")?
                .and_utc()
                .timestamp_millis(),
            Some("UTC".to_string()),
        ),
        EventTime::DateTimeUtc(datetime) => (datetime.timestamp_millis(), Some("UTC".to_string())),
        EventTime::DateTimeFloating(datetime) => (datetime.and_utc().timestamp_millis(), None),
        EventTime::DateTimeZoned { tzid, .. } => {
            (value.to_utc().timestamp_millis(), Some(tzid.clone()))
        }
    };
    let millis = u64::try_from(millis).context("Tuta cannot store dates before 1970")?;
    Ok((DateTime::from_millis(millis), zone))
}

fn recurrence_from_tuta(rule: &CalendarRepeatRule, all_day: bool) -> Result<Recurrence> {
    let frequency = match rule.frequency {
        0 => "DAILY",
        1 => "WEEKLY",
        2 => "MONTHLY",
        3 => "YEARLY",
        value => bail!("Unknown Tuta repeat frequency: {value}"),
    };
    let mut rrule = format!("FREQ={frequency};INTERVAL={}", rule.interval.max(1));
    match rule.endType {
        0 => {}
        1 => rrule.push_str(&format!(
            ";COUNT={}",
            rule.endValue.context("Tuta COUNT rule has no endValue")?
        )),
        2 => {
            let exclusive = rule.endValue.context("Tuta UNTIL rule has no endValue")?;
            let exclusive = ChronoDateTime::<Utc>::from_timestamp_millis(exclusive)
                .context("Tuta UNTIL value is outside chrono's range")?;
            let inclusive = if all_day {
                exclusive - Duration::days(1)
            } else {
                exclusive - Duration::seconds(1)
            };
            let value = if all_day {
                inclusive.format("%Y%m%d").to_string()
            } else {
                inclusive.format("%Y%m%dT%H%M%SZ").to_string()
            };
            rrule.push_str(&format!(";UNTIL={value}"));
        }
        value => bail!("Unknown Tuta repeat end type: {value}"),
    }

    let mut advanced: BTreeMap<i64, Vec<&str>> = BTreeMap::new();
    for item in &rule.advancedRules {
        advanced
            .entry(item.ruleType)
            .or_default()
            .push(item.interval.as_str());
    }
    for (rule_type, values) in advanced {
        rrule.push(';');
        rrule.push_str(by_rule_name(rule_type)?);
        rrule.push('=');
        rrule.push_str(&values.join(","));
    }

    let mut exdates = rule
        .excludedDates
        .iter()
        .map(|date| {
            from_tuta_time(
                date.date,
                (!rule.timeZone.is_empty()).then_some(rule.timeZone.as_str()),
                all_day,
            )
        })
        .collect::<Result<Vec<_>>>()?;
    exdates.sort_by_key(EventTime::to_utc);
    Ok(Recurrence {
        rrule,
        exdates,
        rdates: Vec::new(),
    })
}

fn recurrence_to_tuta(
    recurrence: &Recurrence,
    timezone: &str,
    all_day: bool,
) -> Result<CalendarRepeatRule> {
    if !recurrence.rdates.is_empty() {
        bail!("Tuta does not support RDATE recurrence additions");
    }
    let mut parts = BTreeMap::new();
    for part in recurrence.rrule.split(';') {
        let (key, value) = part
            .split_once('=')
            .with_context(|| format!("Invalid RRULE part: {part}"))?;
        parts.insert(key.to_ascii_uppercase(), value.to_string());
    }
    let frequency = match parts.get("FREQ").map(String::as_str) {
        Some("DAILY") => 0,
        Some("WEEKLY") => 1,
        Some("MONTHLY") => 2,
        Some("YEARLY") => 3,
        Some(value) => bail!("Tuta does not support RRULE frequency {value}"),
        None => bail!("RRULE has no FREQ"),
    };
    let interval = parts
        .get("INTERVAL")
        .map(|value| value.parse::<i64>())
        .transpose()
        .context("Invalid RRULE INTERVAL")?
        .unwrap_or(1);
    let (end_type, end_value) = if let Some(count) = parts.get("COUNT") {
        (
            1,
            Some(count.parse::<i64>().context("Invalid RRULE COUNT")?),
        )
    } else if let Some(until) = parts.get("UNTIL") {
        (2, Some(parse_until(until, timezone, all_day)?))
    } else {
        (0, None)
    };
    let mut advanced_rules = Vec::new();
    for (key, value) in parts {
        let Some(rule_type) = by_rule_type(&key) else {
            continue;
        };
        for interval in value.split(',').filter(|value| !value.is_empty()) {
            advanced_rules.push(AdvancedRepeatRule {
                _id: None,
                ruleType: rule_type,
                interval: interval.to_string(),
                _errors: Default::default(),
            });
        }
    }
    let mut excluded_dates = recurrence
        .exdates
        .iter()
        .map(|date| {
            Ok(DateWrapper {
                _id: None,
                date: to_tuta_time(date)?.0,
                _errors: Default::default(),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    excluded_dates.sort_by_key(|date| date.date.as_millis());
    Ok(CalendarRepeatRule {
        _id: None,
        frequency,
        endType: end_type,
        endValue: end_value,
        interval,
        timeZone: timezone.to_string(),
        excludedDates: excluded_dates,
        advancedRules: advanced_rules,
        _errors: Default::default(),
    })
}

fn parse_until(value: &str, timezone: &str, all_day: bool) -> Result<i64> {
    let inclusive = if all_day || value.len() == 8 {
        let date = NaiveDate::parse_from_str(value, "%Y%m%d").context("Invalid date UNTIL")?;
        date.and_hms_opt(0, 0, 0)
            .context("Invalid date UNTIL")?
            .and_utc()
            + Duration::days(1)
    } else if value.ends_with('Z') {
        NaiveDateTime::parse_from_str(value, "%Y%m%dT%H%M%SZ")
            .context("Invalid UTC UNTIL")?
            .and_utc()
            + Duration::seconds(1)
    } else {
        let local =
            NaiveDateTime::parse_from_str(value, "%Y%m%dT%H%M%S").context("Invalid local UNTIL")?;
        let tz: chrono_tz::Tz = timezone
            .parse()
            .with_context(|| format!("Unknown recurrence timezone: {timezone}"))?;
        tz.from_local_datetime(&local)
            .earliest()
            .context("Invalid local UNTIL in recurrence timezone")?
            .with_timezone(&Utc)
            + Duration::seconds(1)
    };
    Ok(inclusive.timestamp_millis())
}

fn by_rule_name(value: i64) -> Result<&'static str> {
    match value {
        0 => Ok("BYMINUTE"),
        1 => Ok("BYHOUR"),
        2 => Ok("BYDAY"),
        3 => Ok("BYMONTHDAY"),
        4 => Ok("BYYEARDAY"),
        5 => Ok("BYWEEKNO"),
        6 => Ok("BYMONTH"),
        7 => Ok("BYSETPOS"),
        8 => Ok("WKST"),
        value => bail!("Unknown Tuta advanced repeat rule: {value}"),
    }
}

fn by_rule_type(name: &str) -> Option<i64> {
    match name {
        "BYMINUTE" => Some(0),
        "BYHOUR" => Some(1),
        "BYDAY" => Some(2),
        "BYMONTHDAY" => Some(3),
        "BYYEARDAY" => Some(4),
        "BYWEEKNO" => Some(5),
        "BYMONTH" => Some(6),
        "BYSETPOS" => Some(7),
        "WKST" => Some(8),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use caldir_core::XProperty;
    use chrono::TimeZone;

    fn sample_tuta_event() -> CalendarEvent {
        CalendarEvent {
            _id: None,
            _permissions: Default::default(),
            _format: 0,
            _ownerGroup: None,
            _ownerEncSessionKey: None,
            summary: "Standup".to_string(),
            description: "Daily sync".to_string(),
            startTime: DateTime::from_millis(1_784_369_400_000),
            endTime: DateTime::from_millis(1_784_371_200_000),
            location: "Room 1".to_string(),
            uid: Some("standup@example.com".to_string()),
            hashedUid: None,
            sequence: 2,
            invitedConfidentially: None,
            recurrenceId: None,
            _ownerKeyVersion: None,
            sender: None,
            pendingInvitation: None,
            _kdfNonce: None,
            startTimeZone: Some("Europe/London".to_string()),
            endTimeZone: Some("Europe/London".to_string()),
            repeatRule: Some(CalendarRepeatRule {
                _id: None,
                frequency: 1,
                endType: 1,
                endValue: Some(10),
                interval: 1,
                timeZone: "Europe/London".to_string(),
                excludedDates: Vec::new(),
                advancedRules: vec![AdvancedRepeatRule {
                    _id: None,
                    ruleType: 2,
                    interval: "MO".to_string(),
                    _errors: Default::default(),
                }],
                _errors: Default::default(),
            }),
            alarmInfos: Vec::new(),
            attendees: Vec::new(),
            organizer: None,
            _errors: Default::default(),
        }
    }

    #[test]
    fn maps_tuta_fields_and_recurrence() {
        let event = to_caldir_event(&sample_tuta_event()).unwrap();
        assert_eq!(event.uid.as_str(), "standup@example.com");
        assert_eq!(event.summary.as_deref(), Some("Standup"));
        assert_eq!(
            event.recurrence.unwrap().rrule,
            "FREQ=WEEKLY;INTERVAL=1;COUNT=10;BYDAY=MO"
        );
    }

    #[test]
    fn maps_all_day_event() {
        let mut source = sample_tuta_event();
        source.startTime = DateTime::from_millis(
            Utc.with_ymd_and_hms(2026, 7, 18, 0, 0, 0)
                .unwrap()
                .timestamp_millis() as u64,
        );
        source.endTime = DateTime::from_millis(
            Utc.with_ymd_and_hms(2026, 7, 19, 0, 0, 0)
                .unwrap()
                .timestamp_millis() as u64,
        );
        source.repeatRule = None;
        assert!(matches!(
            to_caldir_event(&source).unwrap().start,
            EventTime::Date(_)
        ));
    }

    #[test]
    fn caldir_round_trip_preserves_core_fields() {
        let mut event = Event::new(
            "Planning",
            EventTime::DateTimeZoned {
                datetime: NaiveDate::from_ymd_opt(2026, 7, 18)
                    .unwrap()
                    .and_hms_opt(10, 0, 0)
                    .unwrap(),
                tzid: "Europe/London".to_string(),
            },
        );
        event.uid = EventUid::new("planning@example.com");
        event.end = Some(EventTime::DateTimeZoned {
            datetime: NaiveDate::from_ymd_opt(2026, 7, 18)
                .unwrap()
                .and_hms_opt(11, 0, 0)
                .unwrap(),
            tzid: "Europe/London".to_string(),
        });
        event.recurrence = Some(Recurrence {
            rrule: "FREQ=MONTHLY;INTERVAL=2;BYDAY=MO;BYSETPOS=1".to_string(),
            exdates: vec![EventTime::DateTimeUtc(
                Utc.with_ymd_and_hms(2026, 9, 7, 9, 0, 0).unwrap(),
            )],
            rdates: Vec::new(),
        });
        event
            .x_properties
            .push(XProperty::new("X-LOCAL", "kept-local"));
        let restored = to_caldir_event(&from_caldir_event(&event).unwrap()).unwrap();
        assert_eq!(restored.uid, event.uid);
        assert_eq!(restored.summary, event.summary);
        assert_eq!(
            restored.recurrence.as_ref().unwrap().rrule,
            event.recurrence.as_ref().unwrap().rrule
        );
    }
}
