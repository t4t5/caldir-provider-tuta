use anyhow::{Context, Result, bail};
use base64::Engine;
use serde::{Deserialize, Serialize};
use tutasdk::GeneratedId;
use tutasdk::login::{CredentialType, Credentials};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub base_url: String,
    pub email: String,
    pub user_id: String,
    pub access_token: String,
    pub encrypted_passphrase_key: String,
    pub credential_type: String,
}

impl Session {
    pub fn from_credentials(
        base_url: impl Into<String>,
        email: impl Into<String>,
        credentials: &Credentials,
    ) -> Self {
        Self {
            base_url: base_url.into(),
            email: email.into(),
            user_id: credentials.user_id.to_string(),
            access_token: credentials.access_token.clone(),
            encrypted_passphrase_key: base64::engine::general_purpose::STANDARD
                .encode(&credentials.encrypted_passphrase_key),
            credential_type: match credentials.credential_type {
                CredentialType::Internal => "internal",
                CredentialType::External => "external",
            }
            .to_string(),
        }
    }

    pub fn credentials(&self) -> Result<Credentials> {
        let credential_type = match self.credential_type.as_str() {
            "internal" => CredentialType::Internal,
            "external" => CredentialType::External,
            value => bail!("Unsupported Tuta credential type: {value}"),
        };
        Ok(Credentials {
            login: self.email.clone(),
            user_id: GeneratedId(self.user_id.clone()),
            access_token: self.access_token.clone(),
            encrypted_passphrase_key: base64::engine::general_purpose::STANDARD
                .decode(&self.encrypted_passphrase_key)
                .context("Invalid encrypted_passphrase_key in Tuta session")?,
            credential_type,
        })
    }

    pub(super) fn slug(email: &str, base_url: &str) -> String {
        Self::account_identifier(email, base_url).replace(['/', '\\', ':', '@', '.'], "_")
    }

    pub fn account_identifier(email: &str, base_url: &str) -> String {
        let host = url::Url::parse(base_url)
            .ok()
            .and_then(|url| url.host_str().map(str::to_string))
            .unwrap_or_else(|| "unknown".to_string());
        format!("{email}@{host}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn credentials() -> Credentials {
        Credentials {
            login: "alice@tuta.com".to_string(),
            user_id: GeneratedId("user-id".to_string()),
            access_token: "token".to_string(),
            encrypted_passphrase_key: vec![1, 2, 3],
            credential_type: CredentialType::Internal,
        }
    }

    #[test]
    fn credentials_round_trip() {
        let session = Session::from_credentials(
            "https://mail.tutanota.com",
            "alice@tuta.com",
            &credentials(),
        );
        let restored = session.credentials().unwrap();
        assert_eq!(restored.login, "alice@tuta.com");
        assert_eq!(restored.user_id.to_string(), "user-id");
        assert_eq!(restored.encrypted_passphrase_key, vec![1, 2, 3]);
    }

    #[test]
    fn account_identifier_uses_email_and_host() {
        assert_eq!(
            Session::account_identifier("alice@tuta.com", "https://mail.tutanota.com"),
            "alice@tuta.com@mail.tutanota.com"
        );
    }
}
