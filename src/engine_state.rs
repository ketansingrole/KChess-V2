use std::path::{Path, PathBuf};

use gpui::Context;

use crate::{
    engine::install::{self, InstallResult},
    storage::{self, EngineSettings},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EngineInstallStatus {
    NotInstalled,
    Installing,
    Ready,
    Error,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EngineSourceMode {
    Managed,
    Custom,
    Unavailable,
}

pub struct EngineState {
    base_dir: PathBuf,
    managed_path: PathBuf,
    custom_path: Option<String>,
    installed_release_tag: Option<String>,
    last_error: Option<String>,
    status_message: Option<String>,
    installing: bool,
}

impl EngineState {
    pub fn new(_cx: &mut Context<Self>) -> Self {
        let base_dir =
            install::default_engine_base_dir().unwrap_or_else(|| PathBuf::from(".kchess"));
        let default_managed_path = install::default_managed_engine_path().unwrap_or_else(|| {
            base_dir
                .join("engines")
                .join("stockfish")
                .join("current")
                .join("stockfish")
        });
        let loaded = storage::load_engine_settings();

        Self {
            base_dir,
            managed_path: loaded
                .managed_path
                .as_deref()
                .map(PathBuf::from)
                .unwrap_or(default_managed_path),
            custom_path: loaded.custom_path,
            installed_release_tag: loaded.release_tag,
            last_error: loaded.last_error,
            status_message: None,
            installing: false,
        }
    }

    pub fn install_status(&self) -> EngineInstallStatus {
        if self.installing {
            return EngineInstallStatus::Installing;
        }

        if self.resolved_engine_path().is_some() {
            return EngineInstallStatus::Ready;
        }

        if self.last_error.is_some() {
            return EngineInstallStatus::Error;
        }

        EngineInstallStatus::NotInstalled
    }

    pub fn source_mode(&self) -> EngineSourceMode {
        if self.custom_path().is_some() && self.resolved_custom_engine_path().is_some() {
            EngineSourceMode::Custom
        } else if self.resolved_managed_engine_path().is_some() {
            EngineSourceMode::Managed
        } else {
            EngineSourceMode::Unavailable
        }
    }

    pub fn status_message(&self) -> Option<&str> {
        self.status_message.as_deref()
    }

    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    pub fn managed_path(&self) -> &Path {
        &self.managed_path
    }

    pub fn custom_path(&self) -> Option<&str> {
        self.custom_path.as_deref()
    }

    pub fn custom_path_warning(&self) -> Option<String> {
        let custom = self.custom_path.as_deref()?.trim();
        if custom.is_empty() {
            return None;
        }
        let custom_path = PathBuf::from(custom);
        if is_valid_engine_binary(&custom_path) {
            None
        } else {
            Some("Custom engine path is invalid; using managed engine if available.".to_string())
        }
    }

    pub fn installed_release_tag(&self) -> Option<&str> {
        self.installed_release_tag.as_deref()
    }

    pub fn is_installing(&self) -> bool {
        self.installing
    }

    pub fn resolved_engine_path(&self) -> Option<PathBuf> {
        self.resolved_custom_engine_path()
            .or_else(|| self.resolved_managed_engine_path())
    }

    pub fn set_custom_path(&mut self, custom_path: Option<String>, cx: &mut Context<Self>) {
        self.custom_path = custom_path
            .map(|path| path.trim().to_string())
            .filter(|path| !path.is_empty());
        self.status_message = Some("Engine path saved.".to_string());
        self.last_error = None;

        if let Err(err) = self.persist_settings() {
            self.last_error = Some(err.to_string());
            self.status_message = None;
        }

        cx.notify();
    }

    pub fn clear_custom_path(&mut self, cx: &mut Context<Self>) {
        self.custom_path = None;
        self.status_message = Some("Custom engine path cleared.".to_string());
        self.last_error = None;

        if let Err(err) = self.persist_settings() {
            self.last_error = Some(err.to_string());
            self.status_message = None;
        }

        cx.notify();
    }

    pub fn install_or_update(&mut self, cx: &mut Context<Self>) {
        if self.installing {
            return;
        }

        self.installing = true;
        self.status_message = Some("Downloading latest Stockfish…".to_string());
        self.last_error = None;
        cx.notify();

        let base_dir = self.base_dir.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { install::install_latest_stockfish_macos_arm64(&base_dir) })
                .await;

            let _ = this.update(cx, |state, cx| {
                state.finish_install(result, cx);
            });
        })
        .detach();
    }

    fn finish_install(
        &mut self,
        result: Result<InstallResult, install::InstallError>,
        cx: &mut Context<Self>,
    ) {
        self.installing = false;

        match result {
            Ok(install_result) => {
                self.managed_path = install_result.installed_path;
                self.installed_release_tag = Some(install_result.release_tag);
                self.last_error = None;
                self.status_message = Some("Stockfish installed successfully.".to_string());
            }
            Err(err) => {
                self.last_error = Some(err.to_string());
                self.status_message = None;
            }
        }

        if let Err(err) = self.persist_settings() {
            self.last_error = Some(err.to_string());
            self.status_message = None;
        }

        cx.notify();
    }

    fn resolved_custom_engine_path(&self) -> Option<PathBuf> {
        let custom_path = self.custom_path.as_deref()?.trim();
        if custom_path.is_empty() {
            return None;
        }

        let path = PathBuf::from(custom_path);
        is_valid_engine_binary(&path).then_some(path)
    }

    fn resolved_managed_engine_path(&self) -> Option<PathBuf> {
        is_valid_engine_binary(&self.managed_path).then_some(self.managed_path.clone())
    }

    fn persist_settings(&self) -> Result<(), storage::StorageError> {
        storage::save_engine_settings(&EngineSettings {
            managed_path: Some(self.managed_path.to_string_lossy().to_string()),
            custom_path: self.custom_path.clone(),
            release_tag: self.installed_release_tag.clone(),
            last_error: self.last_error.clone(),
        })
    }
}

fn is_valid_engine_binary(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }

    #[cfg(not(unix))]
    {
        true
    }
}
