//! The facade's settings and the saved settings (`run_facade` arms, moved without changes).

use super::args::{arg, normal};
use super::core_settings;
use crate::core::Core;
use crate::error::{CoreError, Result};
use serde_json::Value;

impl Core {
    /// Answers the facade's settings methods.
    pub(crate) async fn run_settings(&self, method: &str, args: &[Value]) -> Result<Value> {
        match method {
            "loadData" => self.load_data().await,
            "settings" => Ok(core_settings(&self.get_settings().await?)),
            "desktopSettings" => self.get_settings().await,
            "saveSettings" => {
                let settings = normal("assertCoreSettings", &arg(args, 0))?;
                let _writing = self.settings_writes.lock().await;
                let mut merged = self.get_settings().await?;
                if let (Some(target), Some(changes)) =
                    (merged.as_object_mut(), settings.as_object())
                {
                    for (key, value) in changes {
                        target.insert(key.clone(), value.clone());
                    }
                }
                let saved = self.persist_settings(merged).await?;
                Ok(core_settings(&saved))
            }
            // With `base`, the settings the window's edits started from, only what the window
            // changed is written: another window's changes to other settings stay.
            "saveDesktopSettings" => {
                let settings = normal("assertSettings", &arg(args, 0))?;
                let base = match args.get(1) {
                    None | Some(Value::Null) => None,
                    Some(base) => Some(normal("assertSettings", base)?),
                };
                let _writing = self.settings_writes.lock().await;
                let settings = match base {
                    Some(base) => {
                        let mut merged = self.get_settings().await?;
                        if let (Some(target), Some(edited)) =
                            (merged.as_object_mut(), settings.as_object())
                        {
                            for (key, value) in edited {
                                if base.get(key) != Some(value) {
                                    target.insert(key.clone(), value.clone());
                                }
                            }
                        }
                        normal("assertSettings", &merged)?
                    }
                    None => settings,
                };
                self.persist_settings(settings).await
            }
            _ => Err(CoreError::new(format!("Unknown core method {method}."))),
        }
    }
}
