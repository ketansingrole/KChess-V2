//! The variants KChess plays: each one's Lichess key, name, hint and rules.

use serde::{Deserialize, Serialize};

/// A variant KChess plays, by its Lichess key on the wire.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub enum Variant {
    #[default]
    Standard,
    Chess960,
    KingOfTheHill,
    ThreeCheck,
    Antichess,
    Atomic,
    Horde,
    RacingKings,
    Crazyhouse,
}

/// The Lichess variant keys KChess plays, in `Variant::ALL`'s order: the one list every crate
/// checks a variant against.
pub const VARIANTS: &[&str] = &KEYS;

const KEYS: [&str; 9] = {
    let mut keys = [""; 9];
    let mut i = 0;
    while i < Variant::ALL.len() {
        keys[i] = Variant::ALL[i].key();
        i += 1;
    }
    keys
};

impl Variant {
    /// Every variant, standard first, in the order the variant pickers list them.
    pub const ALL: [Variant; 9] = [
        Variant::Standard,
        Variant::Chess960,
        Variant::KingOfTheHill,
        Variant::ThreeCheck,
        Variant::Antichess,
        Variant::Atomic,
        Variant::Horde,
        Variant::RacingKings,
        Variant::Crazyhouse,
    ];

    /// The Lichess key ("kingOfTheHill").
    pub const fn key(self) -> &'static str {
        match self {
            Variant::Standard => "standard",
            Variant::Chess960 => "chess960",
            Variant::KingOfTheHill => "kingOfTheHill",
            Variant::ThreeCheck => "threeCheck",
            Variant::Antichess => "antichess",
            Variant::Atomic => "atomic",
            Variant::Horde => "horde",
            Variant::RacingKings => "racingKings",
            Variant::Crazyhouse => "crazyhouse",
        }
    }

    /// The variant of a Lichess key.
    pub fn from_key(key: &str) -> Option<Variant> {
        Variant::ALL
            .into_iter()
            .find(|variant| variant.key() == key)
    }

    /// The variant a Lichess game is played in: `fromPosition` (or none) is standard rules from
    /// another start; none for a variant KChess does not play.
    pub fn from_lichess(key: Option<&str>) -> Option<Variant> {
        match key {
            None | Some("" | "fromPosition") => Some(Variant::Standard),
            Some(key) => Variant::from_key(key),
        }
    }

    /// Its usual start as a FEN (Chess960's is the standard one).
    pub fn default_fen(self) -> String {
        crate::rules::make_fen(&crate::rules::default_position(self.rules()))
    }

    /// Why a game of it ends when its own goal is reached; none for the variants that end only
    /// as chess does.
    pub const fn win_reason(self) -> Option<&'static str> {
        Some(match self {
            Variant::KingOfTheHill => "King reached the centre",
            Variant::ThreeCheck => "Third check",
            Variant::Antichess => "Lost every piece",
            Variant::Atomic => "King exploded",
            Variant::Horde => "Horde captured",
            Variant::RacingKings => "King reached the eighth rank",
            Variant::Standard | Variant::Chess960 | Variant::Crazyhouse => return None,
        })
    }

    /// The name people and PGN readers know it by ("King of the Hill").
    pub const fn label(self) -> &'static str {
        match self {
            Variant::Standard => "Standard",
            Variant::Chess960 => "Chess960",
            Variant::KingOfTheHill => "King of the Hill",
            Variant::ThreeCheck => "Three-check",
            Variant::Antichess => "Antichess",
            Variant::Atomic => "Atomic",
            Variant::Horde => "Horde",
            Variant::RacingKings => "Racing Kings",
            Variant::Crazyhouse => "Crazyhouse",
        }
    }

    /// Its rules in one line, for a variant picker.
    pub const fn hint(self) -> &'static str {
        match self {
            Variant::Standard => "Normal chess.",
            Variant::Chess960 => "Pieces on the back rank are shuffled; castling still works.",
            Variant::KingOfTheHill => {
                "Win by bringing your king to one of the four centre squares."
            }
            Variant::ThreeCheck => "Win by giving check three times.",
            Variant::Antichess => "Captures are forced; lose all your pieces to win.",
            Variant::Atomic => "Captures explode, removing every piece next to them except pawns.",
            Variant::Horde => "White has 36 pawns and must checkmate; Black must capture them all.",
            Variant::RacingKings => {
                "No checks allowed; the first king to reach the eighth rank wins."
            }
            Variant::Crazyhouse => {
                "Captured pieces change sides and can be dropped back onto the board."
            }
        }
    }

    /// The rules a position follows: Chess960's are standard chess from another start.
    pub const fn rules(self) -> shakmaty::variant::Variant {
        use shakmaty::variant::Variant as Rules;
        match self {
            Variant::Standard | Variant::Chess960 => Rules::Chess,
            Variant::KingOfTheHill => Rules::KingOfTheHill,
            Variant::ThreeCheck => Rules::ThreeCheck,
            Variant::Antichess => Rules::Antichess,
            Variant::Atomic => Rules::Atomic,
            Variant::Horde => Rules::Horde,
            Variant::RacingKings => Rules::RacingKings,
            Variant::Crazyhouse => Rules::Crazyhouse,
        }
    }

    pub fn is_standard(&self) -> bool {
        *self == Variant::Standard
    }

    /// Whether it plays by standard rules (standard chess, and Chess960 with king-takes-rook
    /// castling), which Stockfish analyses.
    pub const fn standard_rules(self) -> bool {
        matches!(self, Variant::Standard | Variant::Chess960)
    }

    /// Fairy-Stockfish's `UCI_Variant`, for the variants Stockfish does not analyse.
    pub const fn fairy_name(self) -> Option<&'static str> {
        Some(match self {
            Variant::Standard | Variant::Chess960 => return None,
            Variant::Crazyhouse => "crazyhouse",
            Variant::Atomic => "atomic",
            Variant::Antichess => "antichess",
            Variant::Horde => "horde",
            Variant::KingOfTheHill => "kingofthehill",
            Variant::RacingKings => "racingkings",
            Variant::ThreeCheck => "3check",
        })
    }

    /// Whether repetition and the fifty-move rule draw a game, as Lichess counts them.
    pub const fn counts_repetition(self) -> bool {
        matches!(
            self,
            Variant::Standard
                | Variant::Chess960
                | Variant::KingOfTheHill
                | Variant::ThreeCheck
                | Variant::Atomic
        )
    }

    /// The variant a PGN `Variant` header names, standard without one; Chess960 for a name of
    /// standard rules that says 960 or Fischer.
    pub fn from_pgn_name(name: Option<&str>) -> Option<Variant> {
        use shakmaty::variant::Variant as Rules;
        Some(match crate::rules::pgn_variant(name)? {
            Rules::Chess => {
                let name = name.unwrap_or_default().to_lowercase();
                if name.contains("960") || name.contains("fischer") {
                    Variant::Chess960
                } else {
                    Variant::Standard
                }
            }
            Rules::Crazyhouse => Variant::Crazyhouse,
            Rules::KingOfTheHill => Variant::KingOfTheHill,
            Rules::ThreeCheck => Variant::ThreeCheck,
            Rules::Antichess => Variant::Antichess,
            Rules::Atomic => Variant::Atomic,
            Rules::Horde => Variant::Horde,
            Rules::RacingKings => Variant::RacingKings,
        })
    }
}

/// Where a game starts: its variant and the position before the first move.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Setup {
    pub variant: Variant,
    pub fen: String,
}

impl Setup {
    /// The ordinary game from the usual start.
    pub fn standard() -> Setup {
        Setup {
            variant: Variant::Standard,
            fen: crate::rules::INITIAL_FEN.into(),
        }
    }

    /// Standard chess from `fen`.
    pub fn standard_at(fen: impl Into<String>) -> Setup {
        Setup {
            variant: Variant::Standard,
            fen: fen.into(),
        }
    }

    /// A game of `variant` from its usual start.
    pub fn of(variant: Variant) -> Setup {
        Setup {
            variant,
            fen: variant.default_fen(),
        }
    }

    pub fn is_standard_start(&self) -> bool {
        self.variant == Variant::Standard && self.fen == crate::rules::INITIAL_FEN
    }

    /// Its position; none when the FEN is not a legal one in its variant.
    pub fn position(&self) -> Option<shakmaty::variant::VariantPosition> {
        crate::rules::setup_start(self.variant, &self.fen)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_variant_reads_back_from_its_key_name_and_wire_value() {
        for variant in Variant::ALL {
            assert_eq!(Variant::from_key(variant.key()), Some(variant));
            assert_eq!(Variant::from_pgn_name(Some(variant.label())), Some(variant));
            assert_eq!(
                serde_json::to_value(variant).unwrap(),
                serde_json::json!(variant.key())
            );
        }
        assert_eq!(VARIANTS[2], "kingOfTheHill");
        assert_eq!(Variant::from_pgn_name(None), Some(Variant::Standard));
        assert_eq!(
            Variant::from_pgn_name(Some("Fischerandom")),
            Some(Variant::Chess960)
        );
        assert_eq!(Variant::from_key("bughouse"), None);
        assert_eq!(Variant::from_lichess(None), Some(Variant::Standard));
        assert_eq!(
            Variant::from_lichess(Some("fromPosition")),
            Some(Variant::Standard)
        );
        assert_eq!(
            Variant::from_lichess(Some("crazyhouse")),
            Some(Variant::Crazyhouse)
        );
        assert_eq!(Variant::from_lichess(Some("bughouse")), None);
        assert_eq!(Variant::Chess960.default_fen(), crate::rules::INITIAL_FEN);
        assert!(Variant::Crazyhouse.default_fen().contains("[]"));
        assert_eq!(Variant::Horde.win_reason(), Some("Horde captured"));
        assert_eq!(Variant::Crazyhouse.win_reason(), None);
    }

    #[test]
    fn a_setup_crosses_as_its_variant_key_and_fen() {
        let setup = Setup::of(Variant::KingOfTheHill);
        let wire = serde_json::to_value(&setup).unwrap();
        assert_eq!(wire["variant"], "kingOfTheHill");
        assert_eq!(serde_json::from_value::<Setup>(wire).unwrap(), setup);
        assert!(Setup::standard().is_standard_start());
        assert!(!Setup::of(Variant::Chess960).is_standard_start());
    }
}
