use anyhow::Result;
use caldir_core::Event;
use caldir_core::provider::ProviderStorage;
use caldir_core::rpc::UpdateEvent;
use tutasdk::GeneratedId;

use crate::constants::PROVIDER_NAME;
use crate::remote_config::TutaRemoteConfig;
use crate::sdk_glue::login;
use crate::session::SessionStore;
use crate::writer;

pub async fn handle(cmd: UpdateEvent) -> Result<Event> {
    let config = TutaRemoteConfig::try_from(&cmd.remote)?;
    let storage = ProviderStorage::for_provider(PROVIDER_NAME)?;
    let session = SessionStore::new(storage.clone()).load(&config.tuta_account)?;
    let sdk = login(&session, &storage).await?;
    writer::update_event(&sdk, &GeneratedId(config.tuta_calendar), cmd.event).await
}
