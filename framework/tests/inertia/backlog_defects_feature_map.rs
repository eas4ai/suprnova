//! The backlog defects round's feature-map item (PAR-049): the Laravel
//! surface (`feature-map/laravel/surface.jsonl`) holds inertia-laravel
//! 3.5.1 and Inertia.js 3.8.0 next to laravel/framework, so the Inertia
//! protocol rows link to the items that implement them.
//!
//! `fmap parity` refuses a Laravel id the surface does not know. Before
//! this fix the surface held laravel/framework alone, so no row of the
//! inertia-protocol group (HD-01, MW-06, RF-16 and the rest) could name
//! its Laravel side, and the changelog bullets were the only link between
//! a row and what shipped.
//!
//! A group's decisions live in `laravel/parity.jsonl` once `fmap
//! parity-import` applies them, and in `laravel/inertia-protocol-parity.jsonl`
//! until then: the import checks every Suprnova id against the Suprnova
//! surface, which is regenerated on its own schedule. These tests read
//! both, so they hold before and after the import.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use serde_json::Value;

/// The inertia-protocol rows (docs/spec/laravel-parity.md, PAR-046 to
/// PAR-056, and RDOC-001 to RDOC-004 and RDOC-006) whose Laravel or
/// Inertia.js side is a surface item. H03 belongs to the
/// security-headers commitment (RDOC-005). HM-05 is Laravel Boost's
/// guideline file, a Blade resource with no class, method or config key
/// behind it, delivered as a manual passage.
const GROUP_ROWS: &[&str] = &[
    "BL-04", "H02", "HD-01", "HD-03", "HD-04", "HD-08", "HD-10", "HD-12", "HD-13", "HM-04",
    "JE-01", "JE-03", "JE-04", "JE-05", "MW-02", "MW-03", "MW-05", "MW-06", "MW-08", "MW-10",
    "MW-11", "P16", "P17", "PR-01", "PR-03", "PR-04", "PR-07", "PR-08", "PR-12", "PR-13", "PT-01",
    "PT-06", "PT-07", "PT-10", "PT-11", "PT-12", "PT-13", "PT-14", "PT-15", "R04", "R06", "R09",
    "RF-01", "RF-02", "RF-03", "RF-05", "RF-07", "RF-08", "RF-09", "RF-11", "RF-12", "RF-13",
    "RF-15", "RF-16", "RF-17", "RF-18", "RF-19", "RS-02", "RS-05", "RS-07",
];

const INERTIA_LARAVEL: &str = "inertiajs/inertia-laravel";
const INERTIA_JS: &str = "inertiajs/inertia";

fn laravel_map() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../feature-map/laravel")
}

/// One JSON value per non-blank line. A missing file reads as no lines, so
/// the pending decisions file may be gone once its decisions are imported.
fn jsonl(name: &str) -> Vec<Value> {
    let path = laravel_map().join(name);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(e) => panic!("read {}: {e}", path.display()),
    };
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            serde_json::from_str(line)
                .unwrap_or_else(|e| panic!("{name}: line is not JSON ({e}): {line}"))
        })
        .collect()
}

/// The surface's records by id.
fn surface() -> BTreeMap<String, Value> {
    jsonl("surface.jsonl")
        .into_iter()
        .map(|record| {
            let id = record["id"]
                .as_str()
                .unwrap_or_else(|| panic!("surface record without an id: {record}"))
                .to_string();
            (id, record)
        })
        .collect()
}

fn package_of(record: &Value) -> &str {
    record["package"].as_str().unwrap_or("")
}

/// The group row ids a decision's note cites, as whole tokens.
fn cited_rows(decision: &Value) -> BTreeSet<&'static str> {
    let note = decision["note"].as_str().unwrap_or("");
    let tokens: BTreeSet<&str> = note
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
        .collect();
    GROUP_ROWS
        .iter()
        .copied()
        .filter(|row| tokens.contains(row))
        .collect()
}

#[test]
fn feature_map_laravel_surface_holds_inertia_laravel_and_inertia_js_records() {
    let surface = surface();
    let mut per_package: BTreeMap<&str, usize> = BTreeMap::new();
    for record in surface.values() {
        *per_package.entry(package_of(record)).or_default() += 1;
    }
    for package in ["laravel/framework", INERTIA_LARAVEL, INERTIA_JS] {
        assert!(
            per_package.get(package).copied().unwrap_or(0) > 0,
            "the Laravel surface holds no {package} record; records per package: {per_package:?}"
        );
    }

    // Each kind of record the two packages contribute, by a known id: a
    // class method and a facade method read through PHP reflection, a
    // macro and a Blade directive the service provider registers, a config
    // key, a helper, an Artisan command, and the client's exported
    // functions and the protocol headers it sends and reads.
    let anchors = [
        ("Inertia\\ResponseFactory::share", INERTIA_LARAVEL, "method"),
        (
            "Inertia\\Inertia::location",
            INERTIA_LARAVEL,
            "facade method",
        ),
        (
            "Inertia\\Middleware::onEmptyResponse",
            INERTIA_LARAVEL,
            "method",
        ),
        (
            "Inertia\\Support\\Header::VERSION",
            INERTIA_LARAVEL,
            "constant",
        ),
        (
            "Illuminate\\Http\\Request::inertia",
            INERTIA_LARAVEL,
            "macro",
        ),
        ("@inertia", INERTIA_LARAVEL, "blade directive"),
        ("<x-inertia::head>", INERTIA_LARAVEL, "blade component"),
        ("config inertia.ssr.enabled", INERTIA_LARAVEL, "config key"),
        ("helper inertia", INERTIA_LARAVEL, "helper"),
        (
            "artisan inertia:start-ssr",
            INERTIA_LARAVEL,
            "artisan command",
        ),
        (
            "@inertiajs/core objectToFormData",
            INERTIA_JS,
            "ts function",
        ),
        (
            "@inertiajs/core mergeDataIntoQueryString",
            INERTIA_JS,
            "ts function",
        ),
        ("header X-Inertia-Version", INERTIA_JS, "protocol header"),
        ("header X-Inertia-Redirect", INERTIA_JS, "protocol header"),
    ];
    for (id, package, kind) in anchors {
        let record = surface
            .get(id)
            .unwrap_or_else(|| panic!("the Laravel surface has no record {id:?}"));
        assert_eq!(package_of(record), package, "{id}: package");
        assert_eq!(record["kind"], kind, "{id}: kind");
        assert!(
            record["file"].as_str().is_some_and(|f| !f.is_empty()),
            "{id}: every record cites the file it came from: {record}"
        );
    }

    // The client reads the version header on a 409 and sends it on every
    // visit; the extractor records both sides.
    let version = &surface["header X-Inertia-Version"]["details"];
    assert!(
        version["sent_at"].as_array().is_some_and(|a| !a.is_empty())
            && version["read_at"].as_array().is_some_and(|a| !a.is_empty()),
        "X-Inertia-Version is sent and read by the client: {version}"
    );

    let meta: Value = serde_json::from_str(
        &std::fs::read_to_string(laravel_map().join("meta.json")).expect("read meta.json"),
    )
    .expect("meta.json is JSON");
    assert_eq!(
        meta["packages"][INERTIA_LARAVEL]["version"], "3.5.1",
        "{meta}"
    );
    assert_eq!(meta["packages"][INERTIA_JS]["version"], "3.8.0", "{meta}");
    assert_eq!(
        meta["records"].as_u64(),
        Some(surface.len() as u64),
        "meta.json counts every record"
    );
}

#[test]
fn feature_map_every_inertia_protocol_row_names_a_surface_id_the_surface_knows() {
    let surface = surface();
    let recorded = jsonl("parity.jsonl");
    let pending = jsonl("inertia-protocol-parity.jsonl");
    for decision in &pending {
        let status = decision["status"].as_str().unwrap_or("");
        let suprnova = decision["suprnova"].as_array().map_or(0, Vec::len);
        match status {
            "shipped" | "diverged" => assert!(
                suprnova > 0,
                "a {status} decision names the Suprnova items: {decision}"
            ),
            "not_built" | "by_design_no" | "not_applicable" => assert_eq!(
                suprnova, 0,
                "a {status} decision names no Suprnova item: {decision}"
            ),
            other => panic!("unknown parity status {other:?}: {decision}"),
        }
    }

    let mut linked: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for decision in recorded.iter().chain(&pending) {
        let rows = cited_rows(decision);
        if rows.is_empty() {
            continue;
        }
        let id = decision["laravel_id"].as_str().unwrap_or("");
        let record = surface.get(id).unwrap_or_else(|| {
            panic!("a decision citing {rows:?} names {id:?}, which the surface does not know")
        });
        let package = package_of(record);
        assert!(
            [INERTIA_LARAVEL, INERTIA_JS, "laravel/framework"].contains(&package),
            "{id}: unexpected package {package:?}"
        );
        for row in rows {
            linked.entry(row).or_default().push(id.to_string());
        }
    }
    let unlinked: Vec<&str> = GROUP_ROWS
        .iter()
        .copied()
        .filter(|row| !linked.contains_key(row))
        .collect();
    assert!(
        unlinked.is_empty(),
        "inertia-protocol rows with no parity decision naming a surface id: {unlinked:?}"
    );
}
