use std::path::PathBuf;

use anyhow::{Context, Result};
use caldir_core::provider::ProviderStorage;

use super::Session;

pub struct SessionStore {
    storage: ProviderStorage,
}

impl SessionStore {
    pub fn new(storage: ProviderStorage) -> Self {
        Self { storage }
    }

    pub fn save(&self, session: &Session) -> Result<()> {
        let path = self.path_for(&session.email, &session.base_url);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!("Failed to create session directory: {}", parent.display())
            })?;
        }
        let contents = toml::to_string_pretty(session).context("Failed to serialize session")?;
        std::fs::write(&path, contents)
            .with_context(|| format!("Failed to write session to {}", path.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
                .with_context(|| format!("Failed to set permissions on {}", path.display()))?;
        }
        Ok(())
    }

    pub fn load(&self, account_identifier: &str) -> Result<Session> {
        let session_dir = self.session_dir();
        if session_dir.exists() {
            for entry in std::fs::read_dir(&session_dir)? {
                let path = entry?.path();
                if path.extension().and_then(|ext| ext.to_str()) != Some("toml") {
                    continue;
                }
                let contents = std::fs::read_to_string(&path)?;
                if let Ok(session) = toml::from_str::<Session>(&contents)
                    && Session::account_identifier(&session.email, &session.base_url)
                        == account_identifier
                {
                    return Ok(session);
                }
            }
        }
        anyhow::bail!(
            "Tuta session for {account_identifier} not found - run `caldir connect tuta` again"
        )
    }

    fn session_dir(&self) -> PathBuf {
        self.storage.root().join("session")
    }

    fn path_for(&self, email: &str, base_url: &str) -> PathBuf {
        self.session_dir()
            .join(format!("{}.toml", Session::slug(email, base_url)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use tutasdk::GeneratedId;
    use tutasdk::login::{CredentialType, Credentials};

    fn sample_session() -> Session {
        Session::from_credentials(
            "https://mail.tutanota.com",
            "alice@tuta.com",
            &Credentials {
                login: "alice@tuta.com".to_string(),
                user_id: GeneratedId("user-id".to_string()),
                access_token: "token".to_string(),
                encrypted_passphrase_key: vec![1, 2, 3],
                credential_type: CredentialType::Internal,
            },
        )
    }

    #[test]
    fn save_and_load_round_trip() {
        let tmp = TempDir::new().unwrap();
        let store = SessionStore::new(ProviderStorage::new(tmp.path()));
        let session = sample_session();
        store.save(&session).unwrap();
        let loaded = store
            .load(&Session::account_identifier(
                &session.email,
                &session.base_url,
            ))
            .unwrap();
        assert_eq!(loaded.access_token, "token");
    }

    #[cfg(unix)]
    #[test]
    fn save_sets_owner_only_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = TempDir::new().unwrap();
        let store = SessionStore::new(ProviderStorage::new(tmp.path()));
        let session = sample_session();
        store.save(&session).unwrap();
        let path = store.path_for(&session.email, &session.base_url);
        assert_eq!(
            std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
