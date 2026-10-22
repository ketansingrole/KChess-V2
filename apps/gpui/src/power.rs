//! Power state for the engine scheduler (`onBattery`): background engine work yields on battery.

/// Whether the computer runs on battery power. Unknown is "no", as on Electron's desktop hosts
/// without a battery.
pub fn on_battery() -> bool {
    false
}
