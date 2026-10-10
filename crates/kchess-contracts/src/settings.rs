//! `core/src/contracts/settings.ts`: the preference keys the core owns.

use ts_rs::Config;

use crate::generate::Item;
use crate::strs;

/// The keys of `CoreSettings`, in `coreSettings` order.
pub const CORE_SETTINGS_KEYS: &[&str] = &[
    "enginePath",
    "engineLevels",
    "reviewAuto",
    "reviewOnBattery",
    "receiveChallenges",
    "onlineChat",
    "correspondencePoll",
    "cloudEval",
    "voiceHistory",
];

pub fn items(_cfg: &Config) -> Vec<Item> {
    vec![Item::Const {
        name: "CORE_SETTINGS_KEYS".into(),
        values: strs(CORE_SETTINGS_KEYS),
        suffix: " satisfies readonly (keyof CoreSettings)[]".into(),
    }]
}
