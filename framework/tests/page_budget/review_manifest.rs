use super::*;

#[test]
fn budget_001_manifest_icons_need_a_matching_modern_entry() {
    let capture = |icons: serde_json::Value| {
        har(vec![
            entry(
                "/",
                200,
                "text/html",
                "<link rel=manifest href=/app.webmanifest>",
                50,
                &[],
            ),
            entry(
                "/app.webmanifest",
                200,
                "application/manifest+json",
                &serde_json::json!({"icons": icons}).to_string(),
                100,
                &[],
            ),
            entry("/icon.png", 200, "image/png", "invalid png", 120_000, &[]),
            entry("/icon.webp", 200, "image/webp", "webp", 120_000, &[]),
        ])
    };
    let png = serde_json::json!({"src":"icon.png","type":"image/png","sizes":"192x192"});
    for modern in [
        None,
        Some(serde_json::json!({"src":"icon.webp","type":"image/webp","sizes":"512x512"})),
        Some(serde_json::json!({"src":"icon.webp","type":"image/jpeg","sizes":"192x192"})),
    ] {
        let mut icons = vec![png.clone()];
        if let Some(icon) = modern {
            icons.push(icon);
        }
        let audit = PageBudget::default()
            .audit_har(&capture(serde_json::json!(icons)))
            .unwrap();
        assert!(
            audit
                .findings
                .iter()
                .any(|f| f.asset.ends_with("/icon.png") && f.rule == "raster-format"),
            "{audit}"
        );
    }
    let audit = PageBudget::default()
        .audit_har(&capture(
            serde_json::json!([png, {"src":"icon.webp","type":"image/webp","sizes":"192x192"}]),
        ))
        .unwrap();
    assert!(
        !audit.findings.iter().any(|f| f.rule == "raster-format"),
        "{audit}"
    );
    assert!(
        audit.findings.iter().any(|f| f.rule == "png-compression"),
        "{audit}"
    );
}
