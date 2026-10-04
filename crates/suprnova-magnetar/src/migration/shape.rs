//! Explicit legacy source-shape vocabulary and confirmation records.

use core::fmt;

use crate::{Error, Result};

/// A legacy database shape recognized by the migration preflight.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SourceShape {
    /// The Torii SeaORM authentication schema.
    Torii,
    /// The Suprnova web template schema.
    SuprnovaWeb,
    /// The Suprnova API template schema with application-owned i64 users.
    SuprnovaApi,
    /// A database already marked as Magnetar-managed.
    Magnetar,
}

impl SourceShape {
    /// Returns the exact CLI value accepted by the source-shape flag.
    pub const fn cli_value(self) -> &'static str {
        match self {
            Self::Torii => "torii",
            Self::SuprnovaWeb => "suprnova-web",
            Self::SuprnovaApi => "suprnova-api",
            Self::Magnetar => "magnetar",
        }
    }

    /// Parses an operator-supplied CLI source-shape value.
    pub fn parse_cli(value: &str) -> Result<Self> {
        match value {
            "torii" => Ok(Self::Torii),
            "suprnova-web" => Ok(Self::SuprnovaWeb),
            "suprnova-api" => Ok(Self::SuprnovaApi),
            "magnetar" => Ok(Self::Magnetar),
            _ => Err(Error::InvalidInput {
                field: "source-shape".to_owned(),
                message: "expected torii|suprnova-web|suprnova-api|magnetar".to_owned(),
            }),
        }
    }
}

impl fmt::Display for SourceShape {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.cli_value())
    }
}

/// A record that the operator explicitly reviewed auto-detection.
///
/// Constructing this value corresponds to receiving the mandatory
/// `--source-shape` CLI flag. The runner refuses it unless both values match a
/// fresh database inspection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShapeConfirmation {
    /// The shape reported to the operator by advisory detection.
    pub detected: SourceShape,
    /// The shape selected explicitly by the operator.
    pub operator_selected: SourceShape,
}
