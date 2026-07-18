use anyhow::Result;
use caldir_core::RemoteConfigParams;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TutaRemoteConfig {
    pub tuta_account: String,
    pub tuta_calendar: String,
}

impl TutaRemoteConfig {
    pub fn new(account: impl Into<String>, calendar: impl Into<String>) -> Self {
        Self {
            tuta_account: account.into(),
            tuta_calendar: calendar.into(),
        }
    }

    pub fn into_remote_config_params(self) -> RemoteConfigParams {
        let mut params = RemoteConfigParams::new();
        params.insert(
            "tuta_account".to_string(),
            toml::Value::String(self.tuta_account),
        );
        params.insert(
            "tuta_calendar".to_string(),
            toml::Value::String(self.tuta_calendar),
        );
        params
    }
}

impl TryFrom<&RemoteConfigParams> for TutaRemoteConfig {
    type Error = anyhow::Error;

    fn try_from(params: &RemoteConfigParams) -> Result<Self> {
        let tuta_account = params
            .get("tuta_account")
            .and_then(toml::Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("Missing required field: tuta_account"))?
            .to_string();
        let tuta_calendar = params
            .get("tuta_calendar")
            .and_then(toml::Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("Missing required field: tuta_calendar"))?
            .to_string();
        Ok(Self {
            tuta_account,
            tuta_calendar,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let original = TutaRemoteConfig::new("alice@tuta.com@mail.tutanota.com", "group-id");
        let restored =
            TutaRemoteConfig::try_from(&original.clone().into_remote_config_params()).unwrap();
        assert_eq!(restored.tuta_account, original.tuta_account);
        assert_eq!(restored.tuta_calendar, original.tuta_calendar);
    }
}
