use anyhow::{Result, bail};
use caldir_core::provider::ProviderStorage;
use caldir_core::rpc::{
    Connect, ConnectResponse, ConnectStepKind, CredentialField, CredentialsData, FieldType,
};

use crate::constants::{DEFAULT_SERVER_URL, PROVIDER_NAME};
use crate::sdk_glue::make_sdk;
use crate::session::{Session, SessionStore};

pub async fn handle(cmd: Connect) -> Result<ConnectResponse> {
    if cmd.data.contains_key("email") {
        let base_url = DEFAULT_SERVER_URL.to_string();
        let email = required_credential(&cmd, "email")?.to_string();
        let passphrase = required_credential(&cmd, "passphrase")?.to_string();
        let storage = ProviderStorage::for_provider(PROVIDER_NAME)?;
        let sdk = make_sdk(&base_url, &storage)?;
        let logged_in = sdk
            .create_session(&email, &passphrase)
            .await
            .map_err(|error| anyhow::anyhow!("Failed to log in to Tuta: {error}"))?;
        let session = Session::from_credentials(&base_url, &email, &logged_in.credentials());
        SessionStore::new(storage).save(&session)?;
        return Ok(ConnectResponse::Done {
            account_identifier: Some(Session::account_identifier(&email, &base_url)),
            calendars: None,
        });
    }

    let fields = vec![
        CredentialField {
            id: "email".to_string(),
            label: "Tuta email".to_string(),
            field_type: FieldType::Text,
            required: true,
            help: None,
        },
        CredentialField {
            id: "passphrase".to_string(),
            label: "Password".to_string(),
            field_type: FieldType::Password,
            required: true,
            help: None,
        },
    ];
    Ok(ConnectResponse::NeedsInput {
        step: ConnectStepKind::Credentials,
        data: serde_json::to_value(CredentialsData { fields })?,
    })
}

fn required_credential<'a>(cmd: &'a Connect, field: &str) -> Result<&'a str> {
    let value = cmd
        .data
        .get(field)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("Missing '{field}' in credentials"))?;
    if value.is_empty() {
        bail!("'{field}' cannot be empty");
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn initial_step_requests_credentials() {
        let response = handle(Connect {
            options: serde_json::Map::new(),
            data: serde_json::Map::new(),
        })
        .await
        .unwrap();
        let value = serde_json::to_value(response).unwrap();
        assert_eq!(value["status"], "needs_input");
        assert_eq!(value["fields"][0]["id"], "email");
        assert_eq!(value["fields"][1]["field_type"], "password");
        assert_eq!(value["fields"].as_array().unwrap().len(), 2);
    }
}
