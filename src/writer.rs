use anyhow::{Context, Result, bail};
use caldir_core::Event;
use crypto_primitives::aes::{Aes256Key, InitializationVector};
use crypto_primitives::key::GenericAesKey;
use crypto_primitives::sha::sha256;
use rand::Rng;
use tutasdk::entities::Entity;
use tutasdk::entities::generated::sys::DateWrapper;
use tutasdk::entities::generated::tutanota::{CalendarEvent, CalendarGroupRoot};
use tutasdk::rest_error::HttpError;
use tutasdk::{ApiCallError, CustomId, GeneratedId, IdTupleCustom, ListLoadDirection, LoggedInSdk};

use crate::alarms::{create_alarms, retain_other_users_alarms};
use crate::constants::{DAYS_SHIFTED_MS, ITEM_UID_PROPERTY, LONG_EVENT_DURATION_MS};
use crate::content::{item_ref, set_item_ref};
use crate::mapping::from_caldir_event;

pub async fn create_event(
    sdk: &LoggedInSdk,
    calendar_id: &GeneratedId,
    mut event: Event,
) -> Result<Event> {
    let mut remote = from_caldir_event(&event)?;
    validate(&remote)?;
    let crypto = sdk.mail_facade().get_crypto_entity_client();
    let group_root: CalendarGroupRoot = crypto
        .load(calendar_id)
        .await
        .context("Failed to load Tuta calendar root")?;
    prepare_and_create(sdk, calendar_id, &group_root, &mut remote).await?;
    create_alarms(sdk, &remote, &event.reminders).await?;

    if let Some(recurrence_id) = remote.recurrenceId {
        exclude_override_on_master(
            sdk,
            &group_root,
            remote.uid.as_deref().unwrap_or_default(),
            recurrence_id,
        )
        .await?;
    }
    let id = remote
        ._id
        .as_ref()
        .context("Created Tuta event has no id")?;
    set_item_ref(&mut event, &id.to_string());
    event.sequence = i32::try_from(remote.sequence).unwrap_or(i32::MAX);
    event.last_modified = None;
    Ok(event)
}

pub async fn update_event(
    sdk: &LoggedInSdk,
    calendar_id: &GeneratedId,
    mut event: Event,
) -> Result<Event> {
    let id = parse_item_ref(&event)
        .context("event has no X-TUTA-ITEM property; run `caldir pull` first")?;
    let crypto = sdk.mail_facade().get_crypto_entity_client();
    let existing: CalendarEvent = crypto
        .load(&id)
        .await
        .context("Failed to load Tuta event")?;
    if !existing.attendees.is_empty() {
        bail!("Refusing to edit a Tuta invitation with attendees");
    }
    let mut remote = from_caldir_event(&event)?;
    validate(&remote)?;
    remote.uid.clone_from(&existing.uid);
    remote.hashedUid.clone_from(&existing.hashedUid);
    remote.sequence = existing.sequence.saturating_add(1);

    let group_root: CalendarGroupRoot = crypto
        .load(calendar_id)
        .await
        .context("Failed to load Tuta calendar root")?;
    let must_recreate = requires_recreate(&existing, &remote, calendar_id);

    if must_recreate {
        sdk.get_entity_client()
            .erase_list_element(&CalendarEvent::type_ref(), id)
            .await
            .context("Failed to remove old Tuta event while rescheduling")?;
        prepare_and_create(sdk, calendar_id, &group_root, &mut remote).await?;
    } else {
        remote.alarmInfos = retain_other_users_alarms(sdk, &existing);
        remote._id = existing._id;
        remote._permissions = existing._permissions;
        remote._ownerGroup = existing._ownerGroup;
        remote._ownerEncSessionKey = existing._ownerEncSessionKey;
        remote._ownerKeyVersion = existing._ownerKeyVersion;
        remote._kdfNonce = existing._kdfNonce;
        remote.invitedConfidentially = existing.invitedConfidentially;
        remote.sender = existing.sender;
        remote.pendingInvitation = existing.pendingInvitation;
        crypto
            .update_instance(remote.clone())
            .await
            .context("Failed to update Tuta event")?;
    }
    create_alarms(sdk, &remote, &event.reminders).await?;

    let new_id = remote
        ._id
        .as_ref()
        .context("Updated Tuta event has no id")?;
    set_item_ref(&mut event, &new_id.to_string());
    event.sequence = i32::try_from(remote.sequence).unwrap_or(i32::MAX);
    event.last_modified = None;
    Ok(event)
}

pub async fn delete_event(sdk: &LoggedInSdk, event: &Event) -> Result<()> {
    let Some(id) = item_ref(event).map(parse_raw_item_ref).transpose()? else {
        return Ok(());
    };
    match sdk
        .get_entity_client()
        .erase_list_element(&CalendarEvent::type_ref(), id)
        .await
    {
        Ok(()) => Ok(()),
        Err(ApiCallError::ServerResponseError {
            source: HttpError::NotFoundError,
        }) => Ok(()),
        Err(error) => Err(error).context("Failed to delete Tuta event"),
    }
}

async fn prepare_and_create(
    sdk: &LoggedInSdk,
    calendar_id: &GeneratedId,
    group_root: &CalendarGroupRoot,
    event: &mut CalendarEvent,
) -> Result<()> {
    let list_id = if is_long_event(event) {
        group_root.longEvents.clone()
    } else {
        group_root.shortEvents.clone()
    };
    event._id = Some(IdTupleCustom::new(
        list_id,
        random_event_element_id(event.startTime.as_millis()),
    ));
    event._ownerGroup = Some(calendar_id.clone());
    event._permissions = Default::default();
    event._kdfNonce = None;
    event.hashedUid = event.uid.as_deref().map(hashed_uid);

    let mut key_bytes = [0_u8; 32];
    rand::rng().fill(&mut key_bytes);
    let session_key =
        GenericAesKey::Aes256(Aes256Key::from_bytes(&key_bytes).expect("32-byte AES key is valid"));
    let mut iv = [0_u8; 16];
    rand::rng().fill(&mut iv);
    let group_key = sdk
        .get_current_sym_group_key(calendar_id)
        .await
        .context("Failed to load Tuta calendar encryption key")?;
    let encrypted = group_key.encrypt_key(&session_key, InitializationVector::from_arr(iv));
    event._ownerEncSessionKey = Some(encrypted.object);
    event._ownerKeyVersion = Some(encrypted.version as i64);

    sdk.mail_facade()
        .get_crypto_entity_client()
        .create_instance(event.clone(), Some(session_key))
        .await
        .context("Failed to create Tuta event")?;
    Ok(())
}

async fn exclude_override_on_master(
    sdk: &LoggedInSdk,
    group_root: &CalendarGroupRoot,
    uid: &str,
    recurrence_id: tutasdk::date::DateTime,
) -> Result<()> {
    let crypto = sdk.mail_facade().get_crypto_entity_client();
    let masters: Vec<CalendarEvent> = crypto
        .load_all(&group_root.longEvents, ListLoadDirection::ASC)
        .await
        .context("Failed to find recurring Tuta master")?;
    let Some(mut master) = masters.into_iter().find(|event| {
        event.uid.as_deref() == Some(uid)
            && event.repeatRule.is_some()
            && event.recurrenceId.is_none()
    }) else {
        eprintln!("caldir-provider-tuta: recurring master for override {uid} was not found");
        return Ok(());
    };
    let rule = master.repeatRule.as_mut().expect("matched repeat rule");
    if !rule
        .excludedDates
        .iter()
        .any(|value| value.date == recurrence_id)
    {
        rule.excludedDates.push(DateWrapper {
            _id: None,
            date: recurrence_id,
            _errors: Default::default(),
        });
        rule.excludedDates
            .sort_by_key(|value| value.date.as_millis());
        master.sequence = master.sequence.saturating_add(1);
        crypto
            .update_instance(master)
            .await
            .context("Failed to exclude altered instance from Tuta master")?;
    }
    Ok(())
}

pub fn is_long_event(event: &CalendarEvent) -> bool {
    event.repeatRule.is_some()
        || event
            .endTime
            .as_millis()
            .saturating_sub(event.startTime.as_millis())
            > LONG_EVENT_DURATION_MS
}

pub fn requires_recreate(
    existing: &CalendarEvent,
    updated: &CalendarEvent,
    target_calendar: &GeneratedId,
) -> bool {
    existing._ownerGroup.as_ref() != Some(target_calendar)
        || existing.startTime != updated.startTime
        || is_long_event(existing) != is_long_event(updated)
}

pub fn element_id_for(start_ms: u64, shift_ms: i64) -> CustomId {
    let shifted = i128::from(start_ms) + i128::from(shift_ms);
    CustomId::from_custom_string(&shifted.max(0).to_string())
}

pub(crate) fn random_event_element_id(start_ms: u64) -> CustomId {
    let min_shift = -DAYS_SHIFTED_MS.min(i64::try_from(start_ms).unwrap_or(i64::MAX));
    element_id_for(
        start_ms,
        rand::rng().random_range(min_shift..DAYS_SHIFTED_MS),
    )
}

pub fn hashed_uid(uid: &str) -> Vec<u8> {
    sha256(uid.as_bytes())
}

fn validate(event: &CalendarEvent) -> Result<()> {
    if event.endTime.as_millis() <= event.startTime.as_millis() {
        bail!("Tuta event end must be after its start");
    }
    if event.startTime.as_millis() == 0 {
        bail!("Tuta event start must be after 1970-01-01");
    }
    Ok(())
}

fn parse_item_ref(event: &Event) -> Result<IdTupleCustom> {
    parse_raw_item_ref(
        event
            .x_property(ITEM_UID_PROPERTY)
            .context("Missing X-TUTA-ITEM")?,
    )
}

fn parse_raw_item_ref(value: &str) -> Result<IdTupleCustom> {
    let (list_id, element_id) = value
        .split_once('/')
        .with_context(|| format!("Invalid X-TUTA-ITEM value: {value}"))?;
    if list_id.is_empty() || element_id.is_empty() {
        bail!("Invalid X-TUTA-ITEM value: {value}");
    }
    Ok(IdTupleCustom::new(
        GeneratedId(list_id.to_string()),
        CustomId(element_id.to_string()),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tutasdk::date::DateTime;

    fn event(duration: u64, recurring: bool) -> CalendarEvent {
        let mut event = from_caldir_event(&Event::new(
            "Test",
            caldir_core::EventTime::DateTimeUtc(
                chrono::DateTime::from_timestamp(1_784_000_000, 0).unwrap(),
            ),
        ))
        .unwrap();
        event.endTime = DateTime::from_millis(event.startTime.as_millis() + duration);
        if recurring {
            event.repeatRule = Some(
                crate::mapping::from_caldir_event(&{
                    let mut source = Event::new(
                        "Recurring",
                        caldir_core::EventTime::DateTimeUtc(
                            chrono::DateTime::from_timestamp(1_784_000_000, 0).unwrap(),
                        ),
                    );
                    source.recurrence = Some(caldir_core::Recurrence::new("FREQ=DAILY"));
                    source
                })
                .unwrap()
                .repeatRule
                .unwrap(),
            );
        }
        event
    }

    #[test]
    fn element_id_matches_base64url_decimal_recipe() {
        assert_eq!(element_id_for(1_000, -15).to_custom_string(), "985");
        assert_eq!(element_id_for(1_000, 15).to_custom_string(), "1015");
    }

    #[test]
    fn classifies_recurring_and_long_events() {
        assert!(!is_long_event(&event(60_000, false)));
        assert!(is_long_event(&event(LONG_EVENT_DURATION_MS + 1, false)));
        assert!(is_long_event(&event(60_000, true)));
    }

    #[test]
    fn recreate_decision_covers_id_affecting_changes() {
        let calendar = GeneratedId("calendar".to_string());
        let mut existing = event(60_000, false);
        existing._ownerGroup = Some(calendar.clone());
        let unchanged = existing.clone();
        assert!(!requires_recreate(&existing, &unchanged, &calendar));

        let mut moved = unchanged.clone();
        moved.startTime = DateTime::from_millis(moved.startTime.as_millis() + 60_000);
        assert!(requires_recreate(&existing, &moved, &calendar));

        let became_long = event(LONG_EVENT_DURATION_MS + 1, false);
        assert!(requires_recreate(&existing, &became_long, &calendar));

        assert!(requires_recreate(
            &existing,
            &unchanged,
            &GeneratedId("other-calendar".to_string())
        ));
    }

    #[test]
    fn hashes_uid_with_sha256() {
        assert_eq!(
            hashed_uid("abc"),
            vec![
                186, 120, 22, 191, 143, 1, 207, 234, 65, 65, 64, 222, 93, 174, 34, 35, 176, 3, 97,
                163, 150, 23, 122, 156, 180, 16, 255, 97, 242, 0, 21, 173,
            ]
        );
    }
}
