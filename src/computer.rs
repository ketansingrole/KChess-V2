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

    pub const fn elo(self) -> u16 {
        match self {
            Self::Low => 1200,
            Self::Medium => 1800,
            Self::High => 2400,
        }
    }

    pub fn display_label(self) -> String {
        format!("{} ({})", self.label(), self.elo())
    }
}
