//! The room left on the disk that holds the profile, so a frontend can warn before a download
//! that would fill it (the game databases grow to gigabytes).

use std::path::Path;

/// The bytes this user may still write on the disk holding `path`, or `None` when the system
/// cannot say. It reads the file system: call it off the async workers.
pub fn free_bytes(path: &Path) -> Option<u64> {
    // The databases folder may not exist yet; its profile does.
    let existing = path.ancestors().find(|at| at.exists())?;
    os::free_bytes(existing)
}

#[cfg(unix)]
mod os {
    use std::path::Path;

    /// The blocks free to unprivileged users, as `df` counts "Available".
    pub fn free_bytes(path: &Path) -> Option<u64> {
        let stats = rustix::fs::statvfs(path).ok()?;
        stats.f_bavail.checked_mul(stats.f_frsize)
    }
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod os {
    use std::os::windows::ffi::OsStrExt as _;
    use std::path::Path;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

    /// The bytes free to this user, after any disk quota.
    pub fn free_bytes(path: &Path) -> Option<u64> {
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
        let mut available = 0u64;
        // SAFETY: `wide` is a NUL-terminated UTF-16 path that outlives the call, `available` is
        // a writable u64, and the two totals the call may skip are null.
        let read = unsafe {
            GetDiskFreeSpaceExW(
                wide.as_ptr(),
                &mut available,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        (read != 0).then_some(available)
    }
}

#[cfg(not(any(unix, windows)))]
mod os {
    pub fn free_bytes(_: &std::path::Path) -> Option<u64> {
        None
    }
}

#[cfg(test)]
mod tests {
    /// This disk has room, and a folder still to be made is measured on the disk it will be on.
    #[test]
    fn the_free_space_of_this_disk_is_read() {
        let dir = tempfile::tempdir().expect("a folder");
        let free = super::free_bytes(dir.path()).expect("the system says");
        assert!(free > 0);
        let later = super::free_bytes(&dir.path().join("databases/not-yet")).expect("its disk");
        assert!(later > 0);
    }
}
