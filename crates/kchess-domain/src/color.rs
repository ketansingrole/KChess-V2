//! A side of the board: shakmaty's `Color`, which the wire and stored documents write as
//! `"white"` or `"black"`.

pub use shakmaty::Color;

pub const fn name(color: Color) -> &'static str {
    match color {
        Color::White => "white",
        Color::Black => "black",
    }
}

pub fn from_name(name: &str) -> Option<Color> {
    match name {
        "white" => Some(Color::White),
        "black" => Some(Color::Black),
        _ => None,
    }
}

/// `#[serde(with = "crate::color::by_name")]` for a `Color` field.
pub mod by_name {
    use serde::{Deserialize, Deserializer, Serializer, de};

    use super::Color;

    pub fn serialize<S: Serializer>(color: &Color, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(super::name(*color))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Color, D::Error> {
        let name = <std::borrow::Cow<'de, str>>::deserialize(deserializer)?;
        super::from_name(&name)
            .ok_or_else(|| de::Error::unknown_variant(&name, &["white", "black"]))
    }
}

/// `#[serde(with = "crate::color::option_by_name")]` for an `Option<Color>` field.
pub mod option_by_name {
    use serde::{Deserialize, Deserializer, Serializer, de};

    use super::Color;

    pub fn serialize<S: Serializer>(
        color: &Option<Color>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match color {
            Some(color) => serializer.serialize_str(super::name(*color)),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<Color>, D::Error> {
        let Some(name) = <Option<std::borrow::Cow<'de, str>>>::deserialize(deserializer)? else {
            return Ok(None);
        };
        super::from_name(&name)
            .map(Some)
            .ok_or_else(|| de::Error::unknown_variant(&name, &["white", "black"]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize, Debug, PartialEq)]
    struct Sides {
        #[serde(with = "by_name")]
        turn: Color,
        #[serde(default, with = "option_by_name")]
        winner: Option<Color>,
    }

    #[test]
    fn a_side_crosses_as_its_name() {
        let sides = Sides {
            turn: Color::Black,
            winner: Some(Color::White),
        };
        let wire = serde_json::json!({ "turn": "black", "winner": "white" });
        assert_eq!(serde_json::to_value(&sides).unwrap(), wire);
        assert_eq!(serde_json::from_value::<Sides>(wire).unwrap(), sides);
        let open: Sides = serde_json::from_value(serde_json::json!({ "turn": "white" })).unwrap();
        assert_eq!(open.winner, None);
        assert!(serde_json::from_value::<Sides>(serde_json::json!({ "turn": "red" })).is_err());
    }
}
