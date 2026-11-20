//! How much memory the computer has, read from the OS: a game review runs as many engines at
//! once as the memory has room for (`engine::scheduler`). A computer that cannot tell reads as
//! unknown, and the processors alone decide.

/// The computer's physical memory in bytes, when the OS tells.
pub fn physical() -> Option<u64> {
    os::physical()
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod os {
    use std::ffi::{c_char, c_int, c_void};

    unsafe extern "C" {
        fn sysctlbyname(
            name: *const c_char,
            old: *mut c_void,
            old_len: *mut usize,
            new: *mut c_void,
            new_len: usize,
        ) -> c_int;
    }

    /// `hw.memsize`: the memory installed, a 64-bit count of bytes.
    pub fn physical() -> Option<u64> {
        let mut bytes: u64 = 0;
        let mut size = std::mem::size_of::<u64>();
        // SAFETY: the name is a terminated string; `bytes` is writable for `size` bytes, which
        // is its own size; nothing is set, so the new value is null and 0 long.
        let read = unsafe {
            sysctlbyname(
                c"hw.memsize".as_ptr(),
                (&raw mut bytes).cast(),
                &raw mut size,
                std::ptr::null_mut(),
                0,
            )
        };
        (read == 0).then_some(bytes)
    }
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod os {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};

    pub fn physical() -> Option<u64> {
        // SAFETY: MEMORYSTATUSEX is plain data, so all zeros is a valid value.
        let mut status: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
        status.dwLength = u32::try_from(std::mem::size_of::<MEMORYSTATUSEX>()).ok()?;
        // SAFETY: `status` is a valid, writable MEMORYSTATUSEX whose length is set, as the call
        // requires.
        let read = unsafe { GlobalMemoryStatusEx(&raw mut status) } != 0;
        read.then_some(status.ullTotalPhys)
    }
}

#[cfg(target_os = "linux")]
mod os {
    pub fn physical() -> Option<u64> {
        super::meminfo_total(&std::fs::read_to_string("/proc/meminfo").ok()?)
    }
}

#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
mod os {
    pub fn physical() -> Option<u64> {
        None
    }
}

/// Linux's `/proc/meminfo` gives the memory as `MemTotal:       16303428 kB`.
#[cfg(any(target_os = "linux", test))]
fn meminfo_total(text: &str) -> Option<u64> {
    let line = text
        .lines()
        .find_map(|line| line.strip_prefix("MemTotal:"))?;
    let kib: u64 = line.trim().strip_suffix("kB")?.trim().parse().ok()?;
    kib.checked_mul(1024)
}

#[cfg(test)]
mod tests {
    use super::{meminfo_total, physical};

    #[test]
    fn the_memory_is_read_on_every_system_kchess_ships_for() {
        // No computer that builds or runs KChess has less than 256 MiB.
        #[cfg(any(target_os = "macos", windows, target_os = "linux"))]
        assert!(physical().is_some_and(|bytes| bytes >= 1 << 28));
        #[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
        assert_eq!(physical(), None);
    }

    #[test]
    fn linux_memory_is_the_total_line_in_bytes() {
        let text = "MemFree:         1024 kB\nMemTotal:       16303428 kB\nSwapTotal: 8 kB\n";
        assert_eq!(meminfo_total(text), Some(16_303_428 * 1024));
        assert_eq!(meminfo_total("MemFree: 1024 kB\n"), None);
        assert_eq!(meminfo_total("MemTotal: many kB\n"), None);
        assert_eq!(meminfo_total("MemTotal: 16303428\n"), None);
    }
}
