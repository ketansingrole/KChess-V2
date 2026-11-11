//! The settings and the stored data.

use kchess_contracts::check::check;
use kchess_contracts::core_api::SettingsApi;
use kchess_contracts::types::{AppData, CoreSettings, Settings};
use kchess_domain::misc::value::J;
use serde_json::Value;

use crate::core::Core;
use crate::error::{CoreError, Result};

fn untyped(settings: &Settings) -> Result<Value> {
    serde_json::to_value(settings).map_err(|cause| CoreError::wrap("settings", cause))
}

impl SettingsApi for Core {
    async fn load_data(&self) -> Result<AppData> {
        self.app_data().await
    }

    async fn stored_settings(&self) -> Result<CoreSettings> {
        Ok(self.get_settings().await?.core)
    }

    async fn desktop_settings(&self) -> Result<Settings> {
        self.get_settings().await
    }

    async fn save_settings(&self, settings: CoreSettings) -> Result<CoreSettings> {
        let _writing = self.settings_writes.lock().await;
        let mut merged = self.get_settings().await?;
        merged.core = settings;
        Ok(self.persist_settings(merged).await?.core)
    }

    // With `base`, the settings the window's edits started from, only what the window changed
    // is written: another window's changes to other settings stay.
    async fn save_desktop_settings(
        &self,
        settings: Settings,
        base: Option<Settings>,
    ) -> Result<Settings> {
        let _writing = self.settings_writes.lock().await;
        let settings = match base {
            Some(base) => {
                let (edited, base) = (untyped(&settings)?, untyped(&base)?);
                let mut merged = untyped(&self.get_settings().await?)?;
                if let (Some(target), Some(edited)) = (merged.as_object_mut(), edited.as_object()) {
                    for (key, value) in edited {
                        if base.get(key) != Some(value) {
                            target.insert(key.clone(), value.clone());
                        }
                    }
                }
                check::<Settings>(J::from(&merged)).map_err(CoreError::new)?
            }
            None => settings,
        };
        self.persist_settings(settings).await
    }
}
