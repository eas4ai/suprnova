//! Programmatic operator guidance for the mandatory migration confirmation.

use super::SourceShape;

/// Stable migration-CLI guidance for a host application's upgrade command.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UpgradeGuide;

impl UpgradeGuide {
    /// Returns the mandatory CLI flag spelling.
    pub const fn confirmation_flag() -> &'static str {
        "--source-shape"
    }

    /// Returns the supported values in CLI presentation order.
    pub const fn supported_source_shapes() -> [SourceShape; 4] {
        [
            SourceShape::Torii,
            SourceShape::SuprnovaWeb,
            SourceShape::SuprnovaApi,
            SourceShape::Magnetar,
        ]
    }

    /// Formats the exact confirmation argument for a detected shape.
    pub fn confirmation_argument(shape: SourceShape) -> String {
        format!("{} {}", Self::confirmation_flag(), shape.cli_value())
    }

    /// Returns the operator-visible warning displayed before an apply.
    pub const fn preflight_warning() -> &'static str {
        "Detection is advisory. Review the source shape and rerun with --source-shape before apply."
    }
}
