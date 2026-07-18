use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result};
use caldir_core::Reminder;
use crypto_primitives::aes::{Aes256Key, InitializationVector};
use crypto_primitives::key::GenericAesKey;
use rand::Rng;
use tutasdk::entities::generated::sys::{
    AlarmInfo, AlarmNotification, AlarmServicePost, CalendarAdvancedRepeatRule, CalendarEventRef,
    NotificationSessionKey, PushIdentifier, RepeatRule, UserAlarmInfo, UserAlarmInfoData,
};
use tutasdk::entities::generated::tutanota::{CalendarEvent, CalendarRepeatRule};
use tutasdk::services::ExtraServiceParams;
use tutasdk::services::generated::sys::AlarmService;
use tutasdk::{GeneratedId, ListLoadDirection, LoggedInSdk};

use crate::writer::random_event_element_id;

const MINUTES_PER_HOUR: i64 = 60;
const MINUTES_PER_DAY: i64 = 24 * MINUTES_PER_HOUR;
const MINUTES_PER_WEEK: i64 = 7 * MINUTES_PER_DAY;
const OPERATION_CREATE: i64 = 0;

pub async fn load_reminders_for_events(
    sdk: &LoggedInSdk,
    events: &[CalendarEvent],
) -> Result<HashMap<String, Vec<Reminder>>> {
    let Some(alarm_list_id) = user_alarm_list_id(sdk) else {
        return Ok(HashMap::new());
    };
    let referenced_ids: HashSet<String> = events
        .iter()
        .flat_map(|event| &event.alarmInfos)
        .filter(|id| id.list_id == alarm_list_id)
        .map(ToString::to_string)
        .collect();
    if referenced_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let crypto = sdk.mail_facade().get_crypto_entity_client();
    let alarms: Vec<UserAlarmInfo> = crypto
        .load_all(&alarm_list_id, ListLoadDirection::ASC)
        .await
        .context("Failed to load Tuta reminders")?;
    let triggers_by_id: HashMap<String, String> = alarms
        .into_iter()
        .filter_map(|alarm| {
            let id = alarm._id?;
            let id = id.to_string();
            referenced_ids
                .contains(&id)
                .then_some((id, alarm.alarmInfo.trigger))
        })
        .collect();

    let mut result = HashMap::new();
    for event in events {
        let Some(event_id) = event._id.as_ref() else {
            continue;
        };
        let mut reminders = Vec::new();
        for alarm_id in &event.alarmInfos {
            if alarm_id.list_id != alarm_list_id {
                continue;
            }
            let Some(trigger) = triggers_by_id.get(&alarm_id.to_string()) else {
                continue;
            };
            match reminder_from_trigger(trigger) {
                Some(reminder) => reminders.push(reminder),
                None => eprintln!(
                    "caldir-provider-tuta: ignoring unsupported reminder trigger {trigger:?}"
                ),
            }
        }
        reminders.sort_by_key(|reminder| Reverse(reminder.minutes_before_start));
        result.insert(event_id.to_string(), reminders);
    }
    Ok(result)
}

pub async fn create_alarms(
    sdk: &LoggedInSdk,
    event: &CalendarEvent,
    reminders: &[Reminder],
) -> Result<()> {
    if reminders.is_empty() {
        return Ok(());
    }
    user_alarm_list_id(sdk).context("Tuta account has no reminder list")?;
    let event_id = event
        ._id
        .as_ref()
        .context("Cannot create reminders for a Tuta event without an id")?;
    let user_id = sdk
        .get_user_id()
        .context("Cannot create reminders for a Tuta user without an id")?;
    let user_group_id = sdk.get_user_group_id();
    let user_group_key = sdk
        .get_current_sym_group_key(&user_group_id)
        .await
        .context("Failed to load the Tuta user encryption key")?;
    let owner_key_version =
        i64::try_from(user_group_key.version).context("Tuta user key version exceeds i64")?;
    let notification_key = random_aes256_key();
    let notification_session_keys = notification_session_keys(sdk, &notification_key).await?;
    let calendar_ref = CalendarEventRef {
        _id: None,
        elementId: event_id.element_id.clone(),
        listId: event_id.list_id.clone(),
    };

    let mut alarm_notifications = Vec::with_capacity(reminders.len());
    let mut user_alarm_info_data = Vec::with_capacity(reminders.len());
    for reminder in reminders {
        let trigger = trigger_from_reminder(*reminder);
        let alarm_identifier = random_event_element_id(current_time_millis()).to_string();
        let alarm_session_key = random_aes256_key();
        let owner_enc_session_key = user_group_key.object.encrypt_key(
            &alarm_session_key,
            InitializationVector::from_arr(random_iv()),
        );
        let encrypted_trigger = alarm_session_key
            .encrypt_data(
                trigger.as_bytes(),
                InitializationVector::from_arr(random_iv()),
            )
            .context("Failed to encrypt Tuta reminder trigger")?;
        let alarm_info = AlarmInfo {
            _id: None,
            trigger: trigger.clone(),
            alarmIdentifier: alarm_identifier.clone(),
            calendarRef: calendar_ref.clone(),
            _errors: Default::default(),
        };

        user_alarm_info_data.push(UserAlarmInfoData {
            _id: None,
            ownerEncSessionKey: owner_enc_session_key,
            ownerKeyVersion: owner_key_version,
            encryptedTrigger: encrypted_trigger,
            alarmIdentifier: alarm_identifier,
            ownerGroup: user_group_id.clone(),
            calendarEventRef: calendar_ref.clone(),
        });
        alarm_notifications.push(AlarmNotification {
            _id: None,
            operation: OPERATION_CREATE,
            summary: event.summary.clone(),
            eventStart: event.startTime,
            eventEnd: event.endTime,
            alarmInfo: alarm_info,
            repeatRule: event.repeatRule.as_ref().map(repeat_rule_for_alarm),
            notificationSessionKeys: notification_session_keys.clone(),
            user: user_id.clone(),
            _errors: Default::default(),
        });
    }

    sdk.get_service_executor()
        .post::<AlarmService>(
            AlarmServicePost {
                _format: 0,
                alarmNotifications: alarm_notifications,
                userAlarmInfoData: user_alarm_info_data,
                _errors: Default::default(),
            },
            ExtraServiceParams {
                session_key: Some(notification_key),
                ..Default::default()
            },
        )
        .await
        .context("Failed to create Tuta reminders")?;
    Ok(())
}

pub fn retain_other_users_alarms(
    sdk: &LoggedInSdk,
    event: &CalendarEvent,
) -> Vec<tutasdk::IdTupleGenerated> {
    let Some(alarm_list_id) = user_alarm_list_id(sdk) else {
        return event.alarmInfos.clone();
    };
    event
        .alarmInfos
        .iter()
        .filter(|id| id.list_id != alarm_list_id)
        .cloned()
        .collect()
}

fn user_alarm_list_id(sdk: &LoggedInSdk) -> Option<GeneratedId> {
    sdk.get_user()
        .alarmInfoList
        .as_ref()
        .map(|list| list.alarms.clone())
}

async fn notification_session_keys(
    sdk: &LoggedInSdk,
    notification_key: &GenericAesKey,
) -> Result<Vec<NotificationSessionKey>> {
    let Some(push_list_id) = sdk
        .get_user()
        .pushIdentifierList
        .as_ref()
        .map(|list| list.list.clone())
    else {
        return Ok(Vec::new());
    };
    let crypto = sdk.mail_facade().get_crypto_entity_client();
    let push_identifiers: Vec<PushIdentifier> = crypto
        .load_all(&push_list_id, ListLoadDirection::ASC)
        .await
        .context("Failed to load Tuta push devices for reminders")?;
    let key_loader = crypto.get_crypto_facade().get_key_loader_facade();
    let mut result = Vec::new();
    for identifier in push_identifiers {
        let Some(id) = identifier._id else {
            continue;
        };
        let owner_group = identifier
            ._ownerGroup
            .context("Tuta push device has no owner group")?;
        let owner_key_version = u64::try_from(
            identifier
                ._ownerKeyVersion
                .context("Tuta push device has no owner key version")?,
        )
        .context("Tuta push device has a negative owner key version")?;
        let owner_enc_session_key = identifier
            ._ownerEncSessionKey
            .context("Tuta push device has no encrypted session key")?;
        let owner_group_key = key_loader
            .load_sym_group_key(&owner_group, owner_key_version, None)
            .await
            .context("Failed to load Tuta push device owner key")?;
        let push_identifier_key = owner_group_key
            .decrypt_aes_key(&owner_enc_session_key)
            .context("Failed to decrypt Tuta push device key")?;
        result.push(NotificationSessionKey {
            _id: None,
            pushIdentifierSessionEncSessionKey: push_identifier_key.encrypt_key(
                notification_key,
                InitializationVector::from_arr(random_iv()),
            ),
            pushIdentifier: id,
        });
    }
    Ok(result)
}

fn reminder_from_trigger(trigger: &str) -> Option<Reminder> {
    let (value, unit) = trigger.split_at(trigger.len().checked_sub(1)?);
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let value: i64 = value.parse().ok()?;
    let multiplier = match unit {
        "M" => 1,
        "H" => MINUTES_PER_HOUR,
        "D" => MINUTES_PER_DAY,
        "W" => MINUTES_PER_WEEK,
        _ => return None,
    };
    value.checked_mul(multiplier).map(Reminder::from_minutes)
}

fn trigger_from_reminder(reminder: Reminder) -> String {
    let minutes = reminder.minutes_before_start.unsigned_abs();
    let hour = u64::try_from(MINUTES_PER_HOUR).expect("positive constant");
    let day = u64::try_from(MINUTES_PER_DAY).expect("positive constant");
    let week = u64::try_from(MINUTES_PER_WEEK).expect("positive constant");
    if minutes != 0 && minutes % week == 0 {
        format!("{}W", minutes / week)
    } else if minutes != 0 && minutes % day == 0 {
        format!("{}D", minutes / day)
    } else if minutes != 0 && minutes % hour == 0 {
        format!("{}H", minutes / hour)
    } else {
        format!("{minutes}M")
    }
}

fn repeat_rule_for_alarm(rule: &CalendarRepeatRule) -> RepeatRule {
    RepeatRule {
        _id: None,
        frequency: rule.frequency,
        endType: rule.endType,
        endValue: rule.endValue,
        interval: rule.interval,
        timeZone: rule.timeZone.clone(),
        excludedDates: rule.excludedDates.clone(),
        advancedRules: rule
            .advancedRules
            .iter()
            .map(|advanced| CalendarAdvancedRepeatRule {
                _id: None,
                ruleType: advanced.ruleType,
                interval: advanced.interval.clone(),
                _errors: Default::default(),
            })
            .collect(),
        _errors: Default::default(),
    }
}

fn random_aes256_key() -> GenericAesKey {
    let mut bytes = [0_u8; 32];
    rand::rng().fill(&mut bytes);
    GenericAesKey::Aes256(Aes256Key::from_bytes(&bytes).expect("32-byte AES key is valid"))
}

fn random_iv() -> [u8; 16] {
    let mut iv = [0_u8; 16];
    rand::rng().fill(&mut iv);
    iv
}

fn current_time_millis() -> u64 {
    u64::try_from(chrono::Utc::now().timestamp_millis()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tuta_alarm_units() {
        assert_eq!(
            reminder_from_trigger("10M"),
            Some(Reminder::from_minutes(10))
        );
        assert_eq!(
            reminder_from_trigger("2H"),
            Some(Reminder::from_minutes(120))
        );
        assert_eq!(
            reminder_from_trigger("3D"),
            Some(Reminder::from_minutes(4_320))
        );
        assert_eq!(
            reminder_from_trigger("1W"),
            Some(Reminder::from_minutes(10_080))
        );
    }

    #[test]
    fn rejects_invalid_tuta_alarm_triggers() {
        assert_eq!(reminder_from_trigger(""), None);
        assert_eq!(reminder_from_trigger("tenM"), None);
        assert_eq!(reminder_from_trigger("-1H"), None);
        assert_eq!(reminder_from_trigger("1S"), None);
        assert_eq!(reminder_from_trigger("1H30M"), None);
    }

    #[test]
    fn formats_reminders_using_largest_exact_tuta_unit() {
        assert_eq!(trigger_from_reminder(Reminder::from_minutes(0)), "0M");
        assert_eq!(trigger_from_reminder(Reminder::from_minutes(10)), "10M");
        assert_eq!(trigger_from_reminder(Reminder::from_minutes(60)), "1H");
        assert_eq!(trigger_from_reminder(Reminder::from_minutes(90)), "90M");
        assert_eq!(trigger_from_reminder(Reminder::from_minutes(2_880)), "2D");
        assert_eq!(trigger_from_reminder(Reminder::from_minutes(20_160)), "2W");
    }
}
