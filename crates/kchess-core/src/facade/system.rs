//! Network usage and the user's themes.

use std::sync::Arc;

use kchess_contracts::core_api::SystemApi;
use kchess_contracts::types::{CustomThemeReport, UsageReport};

use crate::core::Core;
use crate::error::{CoreError, Result};

impl SystemApi for Core {
    async fn usage(&self) -> Result<UsageReport> {
        self.usage.report().await
    }

    async fn reset_usage(&self) -> Result<()> {
        self.usage.reset().await
    }

    async fn custom_themes(&self) -> Result<CustomThemeReport> {
        let (host, data_dir) = (Arc::clone(&self.host), self.data_dir());
        let report =
            tokio::task::spawn_blocking(move || crate::themes::custom_themes(&*host, &data_dir))
                .await
                .map_err(|_| CoreError::new("Custom themes could not be read."))??;
        serde_json::from_value(report).map_err(|cause| CoreError::wrap("custom themes", cause))
    }
}
