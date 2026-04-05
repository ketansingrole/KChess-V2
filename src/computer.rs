use crate::engine::stockfish::EngineProfile;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComputerStrength {
    Low,
    Medium,
    High,
}

impl ComputerStrength {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Low => "Low",
            Self::Medium => "Medium",
            Self::High => "High",
        }
    }

    pub const fn engine_profile(self) -> EngineProfile {
        match self {
            Self::Low => EngineProfile::new(true, Some(1350), 700),
            Self::Medium => EngineProfile::new(true, Some(1800), 1400),
            Self::High => EngineProfile::new(false, None, 2500),
        }
    }

    pub fn display_label(self) -> String {
        match self.engine_profile().elo {
            Some(elo) => format!("{} ({})", self.label(), elo),
            None => format!("{} (Max)", self.label()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strength_profiles_match_expected_values() {
        assert_eq!(
            ComputerStrength::Low.engine_profile(),
            EngineProfile::new(true, Some(1350), 700)
        );
        assert_eq!(
            ComputerStrength::Medium.engine_profile(),
            EngineProfile::new(true, Some(1800), 1400)
        );
        assert_eq!(
            ComputerStrength::High.engine_profile(),
            EngineProfile::new(false, None, 2500)
        );
    }

    #[test]
    fn high_profile_has_no_elo_limit() {
        assert_eq!(ComputerStrength::High.engine_profile().elo, None);
    }
}
