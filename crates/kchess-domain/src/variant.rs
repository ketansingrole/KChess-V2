//! The variants KChess plays: one row each, which every key list, label, PGN name and rules
//! lookup derives from.

use shakmaty::variant::Variant;

/// One playable variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VariantInfo {
    /// The Lichess key, which KChess stores and sends too ("kingOfTheHill").
    pub key: &'static str,
    /// The rules shakmaty plays it by; Chess960 is chess from another start.
    pub rules: Variant,
    /// What the app calls it ("King of the Hill").
    pub label: &'static str,
    /// The PGN `Variant` header other PGN readers know it by.
    pub pgn_name: &'static str,
    /// What sets it apart, in one line.
    pub hint: &'static str,
}

const fn row(
    key: &'static str,
    rules: Variant,
    label: &'static str,
    pgn_name: &'static str,
    hint: &'static str,
) -> VariantInfo {
    VariantInfo {
        key,
        rules,
        label,
        pgn_name,
        hint,
    }
}

/// Every variant, standard first, in the order the variant pickers list them.
pub const VARIANTS: [VariantInfo; 9] = [
    row(
        "standard",
        Variant::Chess,
        "Standard",
        "Standard",
        "Normal chess.",
    ),
    row(
        "chess960",
        Variant::Chess,
        "Chess960",
        "Chess960",
        "Pieces on the back rank are shuffled; castling still works.",
    ),
    row(
        "kingOfTheHill",
        Variant::KingOfTheHill,
        "King of the Hill",
        "King of the Hill",
        "Win by bringing your king to one of the four centre squares.",
    ),
    row(
        "threeCheck",
        Variant::ThreeCheck,
        "Three-check",
        "Three-check",
        "Win by giving check three times.",
    ),
    row(
        "antichess",
        Variant::Antichess,
        "Antichess",
        "Antichess",
        "Captures are forced; lose all your pieces to win.",
    ),
    row(
        "atomic",
        Variant::Atomic,
        "Atomic",
        "Atomic",
        "Captures explode, removing every piece next to them except pawns.",
    ),
    row(
        "horde",
        Variant::Horde,
        "Horde",
        "Horde",
        "White has 36 pawns and must checkmate; Black must capture them all.",
    ),
    row(
        "racingKings",
        Variant::RacingKings,
        "Racing Kings",
        "Racing Kings",
        "No checks allowed; the first king to reach the eighth rank wins.",
    ),
    row(
        "crazyhouse",
        Variant::Crazyhouse,
        "Crazyhouse",
        "Crazyhouse",
        "Captured pieces change sides and can be dropped back onto the board.",
    ),
];

/// The keys of `VARIANTS`, in its order.
pub const KEYS: [&str; VARIANTS.len()] = {
    let mut keys = [""; VARIANTS.len()];
    let mut i = 0;
    while i < VARIANTS.len() {
        keys[i] = VARIANTS[i].key;
        i += 1;
    }
    keys
};

/// The variant with this Lichess key.
pub fn info(key: &str) -> Option<&'static VariantInfo> {
    VARIANTS.iter().find(|variant| variant.key == key)
}

/// `variantFromLichess(key)`: the playable variant of a Lichess key; a game without one, or
/// `fromPosition`, is standard chess.
pub fn from_lichess(key: Option<&str>) -> Option<&'static str> {
    match key {
        None | Some("") | Some("fromPosition") => Some("standard"),
        Some(key) => info(key).map(|variant| variant.key),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lichess_keys_map_to_playable_variants() {
        assert_eq!(from_lichess(None), Some("standard"));
        assert_eq!(from_lichess(Some("fromPosition")), Some("standard"));
        assert_eq!(from_lichess(Some("chess960")), Some("chess960"));
        assert_eq!(from_lichess(Some("crazyhouse")), Some("crazyhouse"));
        assert_eq!(from_lichess(Some("bughouse")), None);
        assert_eq!(KEYS[0], "standard");
        assert_eq!(
            info("threeCheck").map(|v| v.rules),
            Some(Variant::ThreeCheck)
        );
    }

    #[test]
    fn variant_tv_channels_use_the_variant_names() {
        for (key, label) in crate::misc::constants::TV_CHANNELS {
            if let Some(variant) = info(key) {
                assert_eq!(label, variant.label);
            }
        }
    }

    #[test]
    fn pgn_names_read_back_as_the_same_rules() {
        for variant in &VARIANTS {
            assert_eq!(
                crate::rules::pgn_variant(Some(variant.pgn_name)),
                Some(variant.rules),
                "{}",
                variant.key
            );
        }
    }
}
