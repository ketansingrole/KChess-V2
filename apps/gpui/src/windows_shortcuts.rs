//! KChess's shortcuts carry its AppUserModelID (`notifications::APP_ID`). KChess sets that ID on
//! itself so its notifications are its own; a shortcut without it starts a KChess the taskbar
//! counts as another app, so a pinned KChess would show a second button beside the running one.
//! The installer's shortcuts (NSIS has no way to set the ID) and a pin made from one are stamped
//! here, once each, on a background thread at startup.

// The shortcut's property store is reached through COM calls, each with its SAFETY comment.
#![allow(
    unsafe_code,
    reason = "COM's property store calls are unsafe; this module stamps KChess's shortcuts"
)]

use std::path::PathBuf;

use windows::Win32::Storage::EnhancedStorage::PKEY_AppUserModel_ID;
use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};
use windows::Win32::UI::Shell::PropertiesSystem::{
    GPS_READWRITE, IPropertyStore, SHGetPropertyStoreFromParsingName,
};
use windows::core::{BSTR, HSTRING};

/// The shortcuts the installer makes, and the taskbar's copy of one pinned from them.
fn shortcuts() -> Vec<PathBuf> {
    let mut found = Vec::new();
    if let Some(roaming) = std::env::var_os("APPDATA").map(PathBuf::from) {
        found.push(roaming.join(r"Microsoft\Windows\Start Menu\Programs\KChess.lnk"));
        found.push(
            roaming
                .join(r"Microsoft\Internet Explorer\Quick Launch\User Pinned\TaskBar\KChess.lnk"),
        );
    }
    if let Some(desktop) = dirs::desktop_dir() {
        found.push(desktop.join("KChess.lnk"));
    }
    found.into_iter().filter(|path| path.exists()).collect()
}

/// Give each KChess shortcut `id`, where it has another or none.
pub fn stamp(id: &'static str) {
    std::thread::spawn(move || {
        // SAFETY: this thread's own apartment, left before it ends.
        let joined = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok();
        for path in shortcuts() {
            if let Err(cause) = stamp_one(&path, id) {
                crate::log::write(
                    kchess_core::Level::Warn,
                    "shortcuts",
                    &format!("Could not set {}'s app ID: {cause}", path.display()),
                );
            }
        }
        if joined {
            // SAFETY: balances the successful `CoInitializeEx` above, on the same thread.
            unsafe { CoUninitialize() };
        }
    });
}

fn stamp_one(path: &std::path::Path, id: &str) -> windows::core::Result<()> {
    // SAFETY: the path outlives the call; the store is released when dropped.
    let store: IPropertyStore = unsafe {
        SHGetPropertyStoreFromParsingName(&HSTRING::from(path.as_os_str()), None, GPS_READWRITE)
    }?;
    // SAFETY: `PKEY_AppUserModel_ID` is a static key.
    let current = unsafe { store.GetValue(&PKEY_AppUserModel_ID) }?;
    if BSTR::try_from(&current).is_ok_and(|value| value == id) {
        return Ok(());
    }
    let value = PROPVARIANT::from(id);
    // SAFETY: the key and value outlive the calls.
    unsafe {
        store.SetValue(&PKEY_AppUserModel_ID, &value)?;
        store.Commit()
    }
}
