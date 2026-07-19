use std::sync::Arc;

use anyhow::{Context, Result};
use caldir_core::provider::ProviderStorage;
use tutasdk::bindings::native_file_client::NativeFileClient;
use tutasdk::login::LoginError;
use tutasdk::net::native_rest_client::NativeRestClient;
use tutasdk::{LoggedInSdk, Sdk};

use crate::constants::TUTA_SERVER_URL;
use crate::session::Session;

pub fn make_sdk(storage: &ProviderStorage) -> Result<Sdk> {
    let model_cache = storage.root().join("model-cache");
    std::fs::create_dir_all(&model_cache)
        .with_context(|| format!("Failed to create {}", model_cache.display()))?;
    let rest_client = NativeRestClient::try_new().context("Failed to initialize Tuta HTTP")?;
    let file_client =
        NativeFileClient::try_new(model_cache).context("Failed to initialize Tuta model cache")?;
    Ok(Sdk::new(
        TUTA_SERVER_URL.to_string(),
        Arc::new(rest_client),
        Arc::new(file_client),
    ))
}

pub async fn login(session: &Session, storage: &ProviderStorage) -> Result<Arc<LoggedInSdk>> {
    let sdk = make_sdk(storage)?;
    sdk.login(session.credentials()?)
        .await
        .map_err(actionable_login_error)
}

fn actionable_login_error(error: LoginError) -> anyhow::Error {
    match error {
        LoginError::InvalidAccessToken { .. } => {
            anyhow::anyhow!("Tuta session expired - run `caldir connect tuta` again")
        }
        other => anyhow::Error::new(other).context("Failed to resume Tuta session"),
    }
}
