//! The arguments of the position lookups and the opening explorer.

use serde::Deserialize;
use serde_json::Value;

use kchess_domain::color::Color;
use kchess_domain::misc::patterns;
use kchess_domain::misc::value::J;
use kchess_domain::variant::{VARIANTS, Variant};

use super::*;
use crate::types::{
    EXPLORER_SPEEDS, ExplorerSpeed, LookupMode, LookupOptions, POSITION_LOOKUP_KINDS,
    PositionLookupKind,
};

impl Validate for PositionLookupKind {
    fn validate(value: J) -> Result<Self, Refusal> {
        picked(value, POSITION_LOOKUP_KINDS, None)
    }
}

/// The explorer filters as sent: their strings checked one by one, for their messages.
#[derive(Deserialize)]
#[serde(expecting = "an object")]
struct SentFilters {
    speeds: Option<Vec<String>>,
    ratings: Option<Vec<u64>>,
    player: Option<String>,
    color: Option<SentSide>,
    modes: Option<Vec<LookupMode>>,
    since: Option<String>,
    variant: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum SentSide {
    White,
    Black,
}

/// The rating groups of the Lichess opening explorer, in order.
pub const EXPLORER_RATINGS: [u32; 9] = [400, 1000, 1200, 1400, 1600, 1800, 2000, 2200, 2500];

impl Validate for LookupOptions {
    fn validate(value: J) -> Result<Self, Refusal> {
        if matches!(value, J::Undef) {
            return Ok(LookupOptions::default());
        }
        let sent: SentFilters = decode(value, |keys| match keys {
            [] => Some("Invalid explorer filters."),
            _ => None,
        })?;
        let speeds = match sent.speeds {
            Some(speeds) => {
                for speed in &speeds {
                    one_of(speed, EXPLORER_SPEEDS, None, "speeds")?;
                }
                max_items(&speeds, 6, None, "speeds")?;
                let read = speeds.into_iter().map(|speed| {
                    serde_json::from_value::<ExplorerSpeed>(Value::String(speed))
                        .map_err(|error| error.to_string())
                });
                Some(read.collect::<Result<_, _>>()?)
            }
            None => None,
        };
        let ratings = match sent.ratings {
            Some(ratings) => {
                let listed: Vec<String> = EXPLORER_RATINGS.iter().map(u32::to_string).collect();
                let mut kept = Vec::with_capacity(ratings.len());
                for rating in ratings {
                    let known = EXPLORER_RATINGS
                        .into_iter()
                        .find(|known| u64::from(*known) == rating);
                    ensure(known.is_some(), None, "ratings", || {
                        format!("one of {}", listed.join(", "))
                    })?;
                    kept.extend(known);
                }
                max_items(&kept, 9, None, "ratings")?;
                Some(kept)
            }
            None => None,
        };
        if let Some(player) = &sent.player {
            matches(
                player,
                patterns::username,
                None,
                "player",
                "a Lichess username",
            )?;
        }
        if let Some(modes) = &sent.modes {
            max_items(modes, 2, None, "modes")?;
        }
        if let Some(since) = &sent.since {
            matches(
                since,
                patterns::since,
                None,
                "since",
                "a year or a year and month",
            )?;
        }
        let variant = match &sent.variant {
            Some(key) => {
                one_of(key, VARIANTS, Some("Invalid variant."), "variant")?;
                Variant::from_key(key)
            }
            None => None,
        };
        Ok(LookupOptions {
            speeds,
            ratings,
            player: sent.player,
            color: sent.color.map(|side| match side {
                SentSide::White => Color::White,
                SentSide::Black => Color::Black,
            }),
            modes: sent.modes,
            since: sent.since,
            variant,
        })
    }
}

/// `positionLookup`'s arguments together: the position is one of the filters' variant, by that
/// variant's rules.
pub fn position_lookup(
    (_, fen, options): &(PositionLookupKind, Fen, Option<LookupOptions>),
) -> Result<(), Refusal> {
    let variant = options
        .as_ref()
        .and_then(|options| options.variant)
        .filter(|variant| *variant != Variant::Standard);
    analysis_position(&fen.0, variant.map(Variant::key))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obj(fields: Vec<(&str, J)>) -> J {
        J::Obj(fields.into_iter().map(|(k, v)| (k.into(), v)).collect())
    }

    #[test]
    fn explorer_filters_keep_their_messages() {
        let speeds = obj(vec![("speeds", J::Arr(vec![J::Str("bogus".into())]))]);
        assert!(
            check::<LookupOptions>(speeds)
                .unwrap_err()
                .starts_with("speeds: invalid value, expected one of ultraBullet")
        );
        assert_eq!(
            check::<LookupOptions>(J::Str("x".into())),
            Err("Invalid explorer filters.".into())
        );
        let variant = obj(vec![("variant", J::Str("bughouse".into()))]);
        assert_eq!(
            check::<LookupOptions>(variant),
            Err("Invalid variant.".into())
        );
        let ratings = obj(vec![("ratings", J::Arr(vec![J::Num(1234.0)]))]);
        assert!(
            check::<LookupOptions>(ratings)
                .unwrap_err()
                .starts_with("ratings: invalid value, expected one of 400, 1000")
        );
    }
}
