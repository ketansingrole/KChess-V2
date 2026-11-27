//! Lichess rating categories (perfs): a game speed, or a variant other than standard chess. The
//! one list every rule, check and label reads its keys from.

use crate::variant::Variant;

crate::wire_enum!(#[doc = "How fast a game is played, as Lichess groups ratings and explorer games; fastest first."] Speed, SPEEDS {
    UltraBullet = "ultraBullet",
    Bullet = "bullet",
    Blitz = "blitz",
    Rapid = "rapid",
    Classical = "classical",
    Correspondence = "correspondence",
});

impl Speed {
    pub const fn label(self) -> &'static str {
        match self {
            Speed::UltraBullet => "UltraBullet",
            Speed::Bullet => "Bullet",
            Speed::Blitz => "Blitz",
            Speed::Rapid => "Rapid",
            Speed::Classical => "Classical",
            Speed::Correspondence => "Correspondence",
        }
    }

    /// The label of a speed Lichess names by `key`; a key KChess does not know reads as itself.
    pub fn label_of(key: &str) -> &str {
        Speed::from_key(key).map_or(key, |speed| speed.label())
    }
}

/// A Lichess rating category.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Perf {
    Speed(Speed),
    /// A variant other than standard chess, rated on its own at every speed.
    Variant(Variant),
}

impl Perf {
    /// The speeds, then the variants in `Variant::ALL`'s order.
    pub const ALL: [Perf; 14] = [
        Perf::Speed(Speed::UltraBullet),
        Perf::Speed(Speed::Bullet),
        Perf::Speed(Speed::Blitz),
        Perf::Speed(Speed::Rapid),
        Perf::Speed(Speed::Classical),
        Perf::Speed(Speed::Correspondence),
        Perf::Variant(Variant::Chess960),
        Perf::Variant(Variant::KingOfTheHill),
        Perf::Variant(Variant::ThreeCheck),
        Perf::Variant(Variant::Antichess),
        Perf::Variant(Variant::Atomic),
        Perf::Variant(Variant::Horde),
        Perf::Variant(Variant::RacingKings),
        Perf::Variant(Variant::Crazyhouse),
    ];

    pub const fn key(self) -> &'static str {
        match self {
            Perf::Speed(speed) => speed.as_str(),
            Perf::Variant(variant) => variant.key(),
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Perf::Speed(speed) => speed.label(),
            Perf::Variant(variant) => variant.label(),
        }
    }
}

/// Every perf's key, in `Perf::ALL`'s order.
pub const PERF_KEYS: [&str; 14] = {
    let mut keys = [""; 14];
    let mut i = 0;
    while i < keys.len() {
        keys[i] = Perf::ALL[i].key();
        i += 1;
    }
    keys
};

#[cfg(test)]
mod tests {
    use super::*;

    /// Every variant but standard chess is a perf of its own, and no key is listed twice.
    #[test]
    fn the_perfs_are_the_speeds_and_the_other_variants() {
        let variants: Vec<Variant> = Variant::ALL
            .into_iter()
            .filter(|variant| *variant != Variant::Standard)
            .collect();
        let listed: Vec<Variant> = Perf::ALL
            .into_iter()
            .filter_map(|perf| match perf {
                Perf::Variant(variant) => Some(variant),
                Perf::Speed(_) => None,
            })
            .collect();
        assert_eq!(listed, variants);
        let mut keys = PERF_KEYS.to_vec();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), PERF_KEYS.len());
        assert_eq!(&PERF_KEYS[..6], SPEEDS);
        assert_eq!(Speed::from_key("chess960"), None);
    }

    #[test]
    fn a_speed_key_reads_as_its_label() {
        assert_eq!(Speed::label_of("ultraBullet"), "UltraBullet");
        assert_eq!(Speed::label_of("blitz"), "Blitz");
        assert_eq!(Speed::label_of("hyper"), "hyper");
    }

    /// A perf is labelled as its speed or its variant is.
    #[test]
    fn a_perf_reads_as_its_speed_or_variant() {
        let labels: Vec<&str> = Perf::ALL.into_iter().map(Perf::label).collect();
        assert_eq!(
            labels,
            [
                "UltraBullet",
                "Bullet",
                "Blitz",
                "Rapid",
                "Classical",
                "Correspondence",
                "Chess960",
                "King of the Hill",
                "Three-check",
                "Antichess",
                "Atomic",
                "Horde",
                "Racing Kings",
                "Crazyhouse",
            ]
        );
    }
}
