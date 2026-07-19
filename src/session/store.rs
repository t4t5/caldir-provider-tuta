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
        let path = self.path_for(&session.email);
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
        let path = self.path_for(account_identifier);
        let contents = std::fs::read_to_string(&path).with_context(|| {
            format!(
                "Tuta session for {account_identifier} not found - run `caldir connect tuta` again"
            )
        })?;
        toml::from_str(&contents)
            .with_context(|| format!("Failed to read Tuta session from {}", path.display()))
    }

    fn session_dir(&self) -> PathBuf {
        self.storage.root().join("session")
    }

    fn path_for(&self, email: &str) -> PathBuf {
        self.session_dir()
            .join(format!("{}.toml", Session::slug(email)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use tutasdk::GeneratedId;
    use tutasdk::login::{CredentialType, Credentials};

    fn sample_session() -> Session {
        Session::from_credentials(&Credentials {
            login: "alice@tuta.com".to_string(),
            user_id: GeneratedId("user-id".to_string()),
            access_token: "token".to_string(),
            encrypted_passphrase_key: vec![1, 2, 3],
            credential_type: CredentialType::Internal,
        })
    }

    #[test]
    fn save_and_load_round_trip() {
        let tmp = TempDir::new().unwrap();
        let store = SessionStore::new(ProviderStorage::new(tmp.path()));
        let session = sample_session();
        store.save(&session).unwrap();
        let loaded = store.load(&session.email).unwrap();
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
        let path = store.path_for(&session.email);
        assert_eq!(
            std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
