use anyhow::Result;
use caldir_core::Event;
use caldir_core::provider::ProviderStorage;
use caldir_core::rpc::ListEvents;
use chrono::{DateTime, Utc};
use tutasdk::entities::generated::tutanota::{CalendarEvent, CalendarGroupRoot};
use tutasdk::{GeneratedId, ListLoadDirection};

use crate::constants::PROVIDER_NAME;
use crate::content::set_item_ref;
use crate::mapping::to_caldir_event;
use crate::remote_config::TutaRemoteConfig;
use crate::sdk_glue::login;
use crate::session::SessionStore;

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
    let (short_events, long_events): (Vec<CalendarEvent>, Vec<CalendarEvent>) = tokio::try_join!(
        crypto.load_all(&root.shortEvents, ListLoadDirection::ASC),
        crypto.load_all(&root.longEvents, ListLoadDirection::ASC),
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
            if event.last_modified.is_none() {
                eprintln!(
                    "caldir-provider-tuta: event {} has no remote modification time",
                    event.event_instance_id()
                );
            }
            events.push(event);
        }
    }
    Ok(events)
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
}
