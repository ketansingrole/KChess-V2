//! Whether the computer runs on battery power, read from the OS: background engine work yields
//! on battery (`engine::scheduler`, `engine::review`). A computer that cannot tell, such as a
//! desktop with no battery or a sandbox that hides the power supplies, reads as plugged in.

/// Whether the computer runs on battery power now. Cheap, but it reads the OS: call it off the
/// async workers.
pub fn on_battery() -> bool {
    os::on_battery()
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod os {
    use std::ffi::{CStr, c_char, c_void};

    type CFTypeRef = *const c_void;
    type CFStringRef = *const c_void;

    const UTF8: u32 = 0x0800_0100;

    #[link(name = "IOKit", kind = "framework")]
    unsafe extern "C" {
        fn IOPSCopyPowerSourcesInfo() -> CFTypeRef;
        fn IOPSGetProvidingPowerSourceType(snapshot: CFTypeRef) -> CFStringRef;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFStringGetCString(
            string: CFStringRef,
            buffer: *mut c_char,
            size: isize,
            encoding: u32,
        ) -> u8;
        fn CFRelease(object: CFTypeRef);
    }

    /// IOKit names the source the computer draws from: "AC Power", "Battery Power" or "UPS Power".
    pub fn on_battery() -> bool {
        // SAFETY: the snapshot is an owned CF object (the Copy rule), checked for null and
        // released once, after its last use.
        let snapshot = unsafe { IOPSCopyPowerSourcesInfo() };
        if snapshot.is_null() {
            return false;
        }
        // SAFETY: the snapshot is valid until it is released below. The source type it returns
        // is borrowed from it (the Get rule), checked for null and read before the release.
        let source = unsafe { IOPSGetProvidingPowerSourceType(snapshot) };
        let mut name = [0 as c_char; 32];
        let named = !source.is_null()
            // SAFETY: `name` is writable for its whole length, which is the size passed;
            // CFStringGetCString writes a terminated string, or reports that it did not.
            && unsafe { CFStringGetCString(source, name.as_mut_ptr(), name.len() as isize, UTF8) }
                != 0;
        // SAFETY: released once, after the last use of the snapshot and of `source`.
        unsafe { CFRelease(snapshot) };
        // SAFETY: CFStringGetCString succeeded, so `name` holds a terminated string.
        named && unsafe { CStr::from_ptr(name.as_ptr()) }.to_bytes() == b"Battery Power"
    }
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod os {
    use windows_sys::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};

    /// `ACLineStatus` is 0 off the mains, 1 on it and 255 when Windows cannot tell.
    pub fn on_battery() -> bool {
        // SAFETY: SYSTEM_POWER_STATUS is plain data, so all zeros is a valid value.
        let mut status: SYSTEM_POWER_STATUS = unsafe { std::mem::zeroed() };
        // SAFETY: `status` is a valid, writable SYSTEM_POWER_STATUS for the whole call.
        let read = unsafe { GetSystemPowerStatus(&mut status) } != 0;
        read && status.ACLineStatus == 0
    }
}

#[cfg(target_os = "linux")]
mod os {
    pub fn on_battery() -> bool {
        super::supplies_on_battery(std::path::Path::new("/sys/class/power_supply"))
    }
}

#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
mod os {
    pub fn on_battery() -> bool {
        false
    }
}

/// Linux's power supplies (`/sys/class/power_supply`, one folder each): on battery when no
/// charger (a supply that is not a battery) is online and a battery is discharging.
#[cfg(any(target_os = "linux", test))]
fn supplies_on_battery(dir: &std::path::Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    let read = |supply: &std::path::Path, name: &str| {
        std::fs::read_to_string(supply.join(name))
            .map(|text| text.trim().to_string())
            .unwrap_or_default()
    };
    let mut discharging = false;
    for supply in entries.flatten().map(|entry| entry.path()) {
        if read(&supply, "type") == "Battery" {
            discharging |= read(&supply, "status") == "Discharging";
        } else if read(&supply, "online") == "1" {
            return false;
        }
    }
    discharging
}

#[cfg(test)]
mod tests {
    use super::supplies_on_battery;
    use std::path::Path;

    fn supply(dir: &Path, name: &str, files: &[(&str, &str)]) {
        let supply = dir.join(name);
        std::fs::create_dir_all(&supply).unwrap();
        for (file, text) in files {
            std::fs::write(supply.join(file), format!("{text}\n")).unwrap();
        }
    }

    #[test]
    fn a_discharging_battery_with_no_charger_online_is_battery_power() {
        let dir = tempfile::tempdir().unwrap();
        supply(dir.path(), "AC", &[("type", "Mains"), ("online", "0")]);
        supply(
            dir.path(),
            "BAT0",
            &[("type", "Battery"), ("status", "Discharging")],
        );
        assert!(supplies_on_battery(dir.path()));
    }

    #[test]
    fn a_charger_online_or_a_battery_not_discharging_is_mains_power() {
        let dir = tempfile::tempdir().unwrap();
        supply(
            dir.path(),
            "BAT0",
            &[("type", "Battery"), ("status", "Discharging")],
        );
        supply(
            dir.path(),
            "ucsi-source-psy",
            &[("type", "USB"), ("online", "1")],
        );
        assert!(!supplies_on_battery(dir.path()), "a charger is online");

        let full = tempfile::tempdir().unwrap();
        supply(
            full.path(),
            "BAT0",
            &[("type", "Battery"), ("status", "Full")],
        );
        assert!(!supplies_on_battery(full.path()), "the battery is full");
    }

    #[test]
    fn no_power_supplies_is_mains_power() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!supplies_on_battery(dir.path()), "a desktop");
        assert!(!supplies_on_battery(&dir.path().join("missing")), "hidden");
    }
}
