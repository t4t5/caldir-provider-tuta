use anyhow::Result;
use caldir_core::provider::ProviderStorage;
use caldir_core::rpc::DeleteEvent;

use crate::constants::PROVIDER_NAME;
use crate::remote_config::TutaRemoteConfig;
use crate::sdk_glue::login;
use crate::session::SessionStore;
use crate::writer;

pub async fn handle(cmd: DeleteEvent) -> Result<()> {
    let config = TutaRemoteConfig::try_from(&cmd.remote)?;
    if crate::content::item_ref(&cmd.event).is_none() {
        return Ok(());
    }
    let storage = ProviderStorage::for_provider(PROVIDER_NAME)?;
    let session = SessionStore::new(storage.clone()).load(&config.tuta_account)?;
    let sdk = login(&session, &storage).await?;
    writer::delete_event(&sdk, &cmd.event).await
}
