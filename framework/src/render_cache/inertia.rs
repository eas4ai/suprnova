//! Inertia protocol identity uses bounded digests rather than raw header lists.

use sha2::{Digest, Sha256};

use super::VarianceDimension;
use super::middleware::KeyFacts;

fn set(value: Option<&str>) -> Vec<&str> {
    let mut values: Vec<_> = value
        .unwrap_or("")
        .split(',')
        .filter(|v| !v.is_empty())
        .collect();
    values.sort_unstable();
    values.dedup();
    values
}

pub(super) fn dimension(facts: &dyn KeyFacts, dimension: &VarianceDimension) -> String {
    let xhr = facts
        .header("X-Inertia")
        .is_some_and(|v| !v.is_empty() && v != "0");
    let values = match dimension {
        VarianceDimension::Inertia => return if xhr { "json" } else { "document" }.to_owned(),
        VarianceDimension::InertiaPartial => serde_json::json!([
            facts.header("X-Inertia-Partial-Component").unwrap_or(""),
            set(facts.header("X-Inertia-Partial-Data")),
            set(facts.header("X-Inertia-Partial-Except")),
        ]),
        VarianceDimension::InertiaVersion => serde_json::json!([
            crate::inertia::visit_version(),
            facts.header("X-Inertia-Version").unwrap_or("")
        ]),
        VarianceDimension::InertiaProtocol => serde_json::json!([
            set(facts.header("X-Inertia-Except-Once-Props")),
            set(facts.header("X-Inertia-Reset")),
            facts.header("X-Inertia-Error-Bag").unwrap_or(""),
            facts
                .header("X-Inertia-Infinite-Scroll-Merge-Intent")
                .unwrap_or("")
        ]),
        _ => serde_json::Value::Null,
    };
    hex::encode(Sha256::digest(values.to_string().as_bytes()))
}
