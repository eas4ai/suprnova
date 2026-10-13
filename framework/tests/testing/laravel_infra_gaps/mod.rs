//! Laravel infrastructure gaps owned by the testing suite: the date parser
//! read against a frozen clock, and the manual's chapter on AI-assisted
//! development.

pub mod ai_chapter;
#[cfg(feature = "testing")]
pub mod dates;
