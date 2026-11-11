//! The settings and the stored data.

use kchess_contracts::core_api::SettingsApi;
use kchess_contracts::types::Settings;
use kchess_domain::misc::checks;
use serde_json::Value;

use super::args::normal;
use super::{core_data, core_settings};
use crate::core::Core;
use crate::error::{CoreError, Result};

/// Settings as the contracts read them.
fn typed(settings: Value) -> Result<Settings> {
    serde_json::from_value(settings).map_err(|cause| CoreError::wrap("settings", cause))
}

fn untyped(settings: &Settings) -> Result<Value> {
    serde_json::to_value(settings).map_err(|cause| CoreError::wrap("settings", cause))
}

impl SettingsApi for Core {
    async fn load_data(&self) -> Result<Value> {
        core_data(&self.app_data().await?)
    }

    async fn stored_settings(&self) -> Result<Value> {
        core_settings(&self.get_settings().await?)
    }

    async fn desktop_settings(&self) -> Result<Settings> {
        typed(self.get_settings().await?)
    }

    async fn save_settings(&self, settings: Value) -> Result<Value> {
        let _writing = self.settings_writes.lock().await;
        let mut merged = self.get_settings().await?;
        if let (Some(target), Some(changes)) = (merged.as_object_mut(), settings.as_object()) {
            for (key, value) in changes {
                target.insert(key.clone(), value.clone());
            }
        }
        let saved = self.persist_settings(merged).await?;
        core_settings(&saved)
    }

    // With `base`, the settings the window's edits started from, only what the window changed
    // is written: another window's changes to other settings stay.
    async fn save_desktop_settings(
        &self,
        settings: Settings,
        base: Option<Settings>,
    ) -> Result<Settings> {
        let settings = untyped(&settings)?;
        let base = base.as_ref().map(untyped).transpose()?;
        let _writing = self.settings_writes.lock().await;
        let settings = match base {
            Some(base) => {
                let mut merged = self.get_settings().await?;
                if let (Some(target), Some(edited)) = (merged.as_object_mut(), settings.as_object())
                {
                    for (key, value) in edited {
                        if base.get(key) != Some(value) {
                            target.insert(key.clone(), value.clone());
                        }
                    }
                }
                normal(checks::settings, &merged)?
            }
            None => settings,
        };
        typed(self.persist_settings(settings).await?)
    }
}
