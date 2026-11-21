//! The explorer's settings as the profile keeps them (`kchess:explorer`).

use super::*;

/// The explorer's settings, as the profile keeps them. A value no version of the app writes
/// reads as the default, so an old or damaged preference still loads.
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct Prefs {
    #[serde(deserialize_with = "lenient")]
    pub(super) kind: PositionLookupKind,
    #[serde(deserialize_with = "known_speeds")]
    pub(super) speeds: Vec<ExplorerSpeed>,
    pub(super) ratings: Vec<f64>,
    pub(super) player: String,
    #[serde(
        serialize_with = "kchess_domain::color::by_name::serialize",
        deserialize_with = "lenient_color"
    )]
    pub(super) color: Color,
    pub(super) since: String,
    pub(super) follow: bool,
}

impl Default for Prefs {
    fn default() -> Prefs {
        Prefs {
            kind: PositionLookupKind::Opening,
            speeds: vec![
                ExplorerSpeed::Blitz,
                ExplorerSpeed::Rapid,
                ExplorerSpeed::Classical,
            ],
            ratings: Vec::new(),
            player: String::new(),
            color: Color::White,
            since: String::new(),
            follow: false,
        }
    }
}

/// The stored database, or Lichess's games for a name this app does not know.
fn lenient<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<PositionLookupKind, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(serde_json::from_value(value).unwrap_or(PositionLookupKind::Opening))
}

/// The stored speeds this app knows.
fn known_speeds<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<ExplorerSpeed>, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|speed| serde_json::from_value(speed.clone()).ok())
        .collect())
}

fn lenient_color<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Color, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(value
        .as_str()
        .and_then(kchess_domain::color::from_name)
        .unwrap_or(Color::White))
}

impl Prefs {
    /// The filters the chosen database takes; an unset one is left out.
    pub(super) fn options(&self) -> LookupOptions {
        let since = (!self.since.is_empty()).then(|| self.since.clone());
        let kind = self.kind;
        let options = match kind {
            PositionLookupKind::Opening => LookupOptions {
                speeds: Some(self.speeds.clone()),
                ratings: Some(self.ratings.iter().map(|rating| *rating as u32).collect()),
                ..LookupOptions::default()
            },
            PositionLookupKind::Player => LookupOptions {
                player: Some(self.player.trim().to_string()),
                color: Some(self.color),
                speeds: Some(self.speeds.clone()),
                ..LookupOptions::default()
            },
            _ => LookupOptions::default(),
        };
        let since = match kind {
            PositionLookupKind::Masters => {
                since.map(|since| since.chars().take(4).collect::<String>())
            }
            PositionLookupKind::Tablebase => None,
            _ => since,
        };
        LookupOptions { since, ..options }
    }
}

pub(super) fn toggled<T: PartialEq + Clone>(list: &[T], value: &T) -> Vec<T> {
    if list.contains(value) {
        list.iter()
            .filter(|entry| *entry != value)
            .cloned()
            .collect()
    } else {
        list.iter().cloned().chain([value.clone()]).collect()
    }
}
