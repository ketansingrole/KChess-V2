//! One live core per profile directory: within this process by a map of claimed profiles, and
//! across processes (a second app launch, the CLI on the same data dir) by an OS lock on a file
//! in the profile.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};

use crate::error::{CoreError, Result};

/// The profiles in use by a live core, by canonical data directory; each holds the claim's token,
/// so a stale release never frees a profile a later core has claimed.
static PROFILES: LazyLock<Mutex<HashMap<PathBuf, u64>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static CLAIMS: AtomicU64 = AtomicU64::new(1);

/// The lock file a live core holds in its profile directory.
const PROFILE_LOCK: &str = "profile.lock";

/// One core's claim on its profile directory.
pub struct ProfileClaim {
    path: PathBuf,
    token: u64,
    /// The lock file, held open: `release` (or drop) frees the OS advisory lock.
    lock: Mutex<Option<std::fs::File>>,
}

impl ProfileClaim {
    /// Claims the profile at `data_dir` (its real path), or fails when a core already runs there.
    pub fn claim(data_dir: &Path) -> Result<ProfileClaim> {
        let path = std::fs::canonicalize(data_dir)
            .map_err(|cause| CoreError::wrap("profile path", cause))?;
        let token = CLAIMS.fetch_add(1, Ordering::SeqCst);
        let mut profiles = PROFILES
            .lock()
            .map_err(|_| CoreError::new("The profiles are unavailable."))?;
        if profiles.contains_key(&path) {
            return Err(CoreError::new(
                "A KChess core is already running for this profile.",
            ));
        }
        // The in-process map cannot see a second process, so the claim also holds an OS
        // advisory lock on a lock file in the data dir for the core's lifetime (`flock` on
        // Unix, `LockFileEx` on Windows: both conflict across open file descriptions, which
        // is exactly how a second process opens the same file).
        let lock = std::fs::File::create(path.join(PROFILE_LOCK))
            .map_err(|cause| CoreError::wrap("profile lock", cause))?;
        if let Err(cause) = lock.try_lock() {
            return Err(match cause {
                std::fs::TryLockError::WouldBlock => {
                    CoreError::new("A KChess core is already running for this profile.")
                }
                std::fs::TryLockError::Error(locked) => CoreError::wrap("profile lock", locked),
            });
        }
        profiles.insert(path.clone(), token);
        Ok(ProfileClaim {
            path,
            token,
            lock: Mutex::new(Some(lock)),
        })
    }

    /// Frees the profile, once the core has closed it.
    pub fn release(&self) {
        if let Ok(mut profiles) = PROFILES.lock()
            && profiles.get(&self.path) == Some(&self.token)
        {
            profiles.remove(&self.path);
        }
        // Free the OS lock now, not at drop: a closed core must be reopenable
        // while its (shut down) object still exists.
        if let Ok(mut held) = self.lock.lock()
            && let Some(file) = held.take()
        {
            let _ = file.unlock();
        }
    }
}

impl Drop for ProfileClaim {
    fn drop(&mut self) {
        self.release();
    }
}
