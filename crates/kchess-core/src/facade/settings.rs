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
            "saveDesktopSettings" => {
                let settings = normal("assertSettings", &arg(args, 0))?;
                self.persist_settings(settings).await
            }
            _ => Err(CoreError::new(format!("Unknown core method {method}."))),
        }
    }
}
