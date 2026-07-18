use anyhow::{Context, Result, bail};
use caldir_core::Event;
use caldir_core::provider::ProviderStorage;
use caldir_core::rpc::ListEvents;
use chrono::{DateTime, Utc};
use tutasdk::entities::generated::tutanota::{CalendarEvent, CalendarGroupRoot};
use tutasdk::util::first_bigger_than_second_custom_id;
use tutasdk::{CustomId, GeneratedId, ListLoadDirection, crypto_entity_client::CryptoEntityClient};

use crate::constants::{DAYS_SHIFTED_MS, PROVIDER_NAME};
use crate::content::set_item_ref;
use crate::mapping::to_caldir_event;
use crate::remote_config::TutaRemoteConfig;
use crate::sdk_glue::login;
use crate::session::SessionStore;
use crate::writer::element_id_for;

const PAGE_SIZE: usize = 200;

pub async fn handle(cmd: ListEvents) -> Result<Vec<Event>> {
    let config = TutaRemoteConfig::try_from(&cmd.remote)?;
    let from = DateTime::parse_from_rfc3339(&cmd.from)?.with_timezone(&Utc);
    let to = DateTime::parse_from_rfc3339(&cmd.to)?.with_timezone(&Utc);
    let storage = ProviderStorage::for_provider(PROVIDER_NAME)?;
    let session = SessionStore::new(storage.clone()).load(&config.tuta_account)?;
    let sdk = login(&session, &storage).await?;
    let calendar_id = GeneratedId(config.tuta_calendar);
    let crypto = sdk.mail_facade().get_crypto_entity_client();
    let root: CalendarGroupRoot = crypto.load(&calendar_id).await?;
    let (short_start, short_end) = short_event_bounds(from, to);
    let (short_events, long_events): (Vec<CalendarEvent>, Vec<CalendarEvent>) = tokio::try_join!(
        load_event_list(&crypto, &root.shortEvents, short_start, Some(&short_end),),
        load_event_list(&crypto, &root.longEvents, CustomId(String::new()), None,),
    )?;
    let mut events = Vec::new();
    for source in short_events.into_iter().chain(long_events) {
        let Some(id) = source._id.as_ref() else {
            eprintln!("caldir-provider-tuta: skipping event without an entity id");
            continue;
        };
        let mut event = match to_caldir_event(&source) {
            Ok(event) => event,
            Err(error) => {
                eprintln!("caldir-provider-tuta: skipping malformed Tuta event {id}: {error}");
                continue;
            }
        };
        set_item_ref(&mut event, &id.to_string());
        if event.recurrence.is_some() || event.occurs_in_range(from, to) {
            events.push(event);
        }
    }
    Ok(events)
}

async fn load_event_list(
    crypto: &CryptoEntityClient,
    list_id: &GeneratedId,
    mut cursor: CustomId,
    upper_bound: Option<&CustomId>,
) -> Result<Vec<CalendarEvent>> {
    let mut events = Vec::new();
    loop {
        let mut page: Vec<CalendarEvent> = crypto
            .load_range(list_id, &cursor, PAGE_SIZE, ListLoadDirection::ASC)
            .await
            .with_context(|| format!("Failed to load Tuta event list {list_id}"))?;
        if page.is_empty() {
            break;
        }
        let last_id = page
            .last()
            .and_then(|event| event._id.as_ref())
            .context("Tuta event list page ends with an event without an id")?;
        let next_cursor = last_id.element_id.clone();
        if next_cursor == cursor {
            bail!("Tuta event list pagination did not advance");
        }
        let reached_end =
            upper_bound.is_some_and(|end| first_bigger_than_second_custom_id(&next_cursor, end));
        events.append(&mut page);
        if reached_end {
            break;
        }
        cursor = next_cursor;
    }
    Ok(events)
}

fn short_event_bounds(from: DateTime<Utc>, to: DateTime<Utc>) -> (CustomId, CustomId) {
    let from_ms = u64::try_from(from.timestamp_millis()).unwrap_or(0);
    let to_ms = u64::try_from(to.timestamp_millis()).unwrap_or(0);
    (
        element_id_for(from_ms, -DAYS_SHIFTED_MS),
        element_id_for(to_ms, DAYS_SHIFTED_MS),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use caldir_core::EventTime;

    #[test]
    fn range_boundaries_are_end_exclusive() {
        let event = Event::new(
            "Boundary",
            EventTime::DateTimeUtc(
                DateTime::parse_from_rfc3339("2026-08-01T00:00:00Z")
                    .unwrap()
                    .with_timezone(&Utc),
            ),
        );
        let from = DateTime::parse_from_rfc3339("2026-07-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let to = DateTime::parse_from_rfc3339("2026-08-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert!(!event.occurs_in_range(from, to));
    }

    #[test]
    fn short_event_bounds_allow_for_randomized_ids() {
        let from = DateTime::parse_from_rfc3339("2026-07-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let to = DateTime::parse_from_rfc3339("2026-08-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        let (start, end) = short_event_bounds(from, to);

        assert_eq!(
            start.to_custom_string(),
            (from.timestamp_millis() - DAYS_SHIFTED_MS).to_string()
        );
        assert_eq!(
            end.to_custom_string(),
            (to.timestamp_millis() + DAYS_SHIFTED_MS).to_string()
        );
    }

    #[test]
    fn short_event_start_saturates_at_unix_epoch() {
        let from = DateTime::parse_from_rfc3339("1969-12-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let to = DateTime::parse_from_rfc3339("1970-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        let (start, _) = short_event_bounds(from, to);

        assert_eq!(start.to_custom_string(), "0");
    }
}
