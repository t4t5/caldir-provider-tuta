use anyhow::Result;
use caldir_core::provider::ProviderStorage;
use caldir_core::rpc::ListCalendars;
use caldir_core::{CalendarConfig, ProviderSlug, RemoteConfig};
use tutasdk::date::calendar_facade::BIRTHDAY_CALENDAR_BASE_ID;

use crate::constants::PROVIDER_NAME;
use crate::content::normalize_color;
use crate::remote_config::TutaRemoteConfig;
use crate::sdk_glue::login;
use crate::session::SessionStore;

pub async fn handle(cmd: ListCalendars) -> Result<Vec<CalendarConfig>> {
    let storage = ProviderStorage::for_provider(PROVIDER_NAME)?;
    let session = SessionStore::new(storage.clone()).load(&cmd.account_identifier)?;
    let sdk = login(&session, &storage).await?;
    let mut render_data: Vec<_> = sdk
        .calendar_facade()
        .get_calendars_render_data()
        .await?
        .into_iter()
        .filter(|(id, _)| !id.as_str().contains(BIRTHDAY_CALENDAR_BASE_ID))
        .collect();
    render_data.sort_by(|(left, _), (right, _)| left.as_str().cmp(right.as_str()));

    Ok(render_data
        .into_iter()
        .map(|(calendar_id, data)| {
            let params = TutaRemoteConfig::new(&cmd.account_identifier, calendar_id.to_string())
                .into_remote_config_params();
            CalendarConfig::new(
                (!data.name.is_empty()).then_some(data.name),
                Some(normalize_color(&data.color)),
                Some(false),
                Some(RemoteConfig::new(ProviderSlug::from(PROVIDER_NAME), params)),
            )
        })
        .collect())
}
