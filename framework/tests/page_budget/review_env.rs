use super::*;

#[test]
fn budget_001_command_reads_environment_configuration() {
    if std::env::var_os("BUDGET_ENV_CHILD").is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "review_env::budget_001_command_reads_environment_configuration",
                "--nocapture",
            ])
            .env("BUDGET_ENV_CHILD", "1")
            .env("PAGE_BUDGET_TOTAL_BYTES", "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        file.path(),
        har(vec![entry("/", 200, "text/html", "hello", 5, &[])]),
    )
    .unwrap();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let result = suprnova::console::test(["budget:audit", file.path().to_str().unwrap()])
                .run()
                .await;
            result.assert_exit_code(1);
            assert!(
                result.output().contains("total-transfer"),
                "{}",
                result.output()
            );
        });
}

const ENV_VALUES: &[(&str, &str)] = &[
    ("PAGE_BUDGET_TOTAL_BYTES", "2"),
    ("PAGE_BUDGET_ASSET_BYTES", "3"),
    ("PAGE_BUDGET_RASTER_THRESHOLD_BYTES", "4"),
    ("PAGE_BUDGET_PNG_MARGIN_PERCENT", "5"),
    ("PAGE_BUDGET_FONT_BYTES", "6"),
    ("PAGE_BUDGET_REQUIRE_IMMUTABLE", "false"),
    ("PAGE_BUDGET_FORBID_ASSET_SET_COOKIE", "false"),
    ("PAGE_BUDGET_FORBID_ASSET_REQUEST_COOKIE", "false"),
    ("PAGE_BUDGET_REQUIRE_COOKIE_INDEPENDENT", "false"),
    ("PAGE_BUDGET_FORBID_FIRST_REDIRECT", "false"),
    ("PAGE_BUDGET_FINGERPRINT_PATTERN", "[0-9]+$"),
    (
        "PAGE_BUDGET_ALLOWED_THIRD_PARTY_HOSTS",
        "cdn.example, fonts.example",
    ),
    ("PAGE_BUDGET_MAX_RESOURCES", "7"),
    ("PAGE_BUDGET_MAX_RESPONSE_BYTES", "8"),
    ("PAGE_BUDGET_REQUEST_TIMEOUT_SECS", "9"),
    ("PAGE_BUDGET_COLLECTION_TIMEOUT_SECS", "10"),
];

#[test]
fn budget_001_from_env_overrides_defaults_and_rejects_each_invalid_field() {
    if let Ok(case) = std::env::var("BUDGET_CONFIG_CASE") {
        match case.as_str() {
            "defaults" => assert_eq!(
                format!("{:?}", PageBudget::from_env().unwrap()),
                format!("{:?}", PageBudget::default())
            ),
            "all" => {
                let budget = PageBudget::from_env().unwrap();
                assert_eq!(budget.total_transfer_bytes, 2);
                assert_eq!(budget.asset_bytes, 3);
                assert_eq!(budget.raster_threshold_bytes, 4);
                assert_eq!(budget.png_margin_percent, 5);
                assert_eq!(budget.font_bytes, 6);
                assert!(!budget.require_immutable);
                assert!(!budget.forbid_asset_set_cookie);
                assert!(!budget.forbid_asset_request_cookie);
                assert!(!budget.require_cookie_independent);
                assert!(!budget.forbid_first_redirect);
                assert_eq!(budget.fingerprint_pattern, "[0-9]+$");
                assert_eq!(
                    budget.allowed_third_party_hosts.unwrap(),
                    ["cdn.example".to_owned(), "fonts.example".to_owned()]
                        .into_iter()
                        .collect()
                );
                assert_eq!(budget.max_resources, 7);
                assert_eq!(budget.max_response_bytes, 8);
                assert_eq!(budget.request_timeout, std::time::Duration::from_secs(9));
                assert_eq!(
                    budget.collection_timeout,
                    std::time::Duration::from_secs(10)
                );
            }
            "empty-hosts" => assert_eq!(
                PageBudget::from_env().unwrap().allowed_third_party_hosts,
                Some(Default::default())
            ),
            name => {
                let error = PageBudget::from_env().unwrap_err().to_string();
                assert!(error.contains(name), "{error} must name {name}");
            }
        }
        return;
    }
    for case in std::iter::once("defaults")
        .chain(std::iter::once("all"))
        .chain(std::iter::once("empty-hosts"))
        .chain(ENV_VALUES.iter().map(|(name, _)| *name))
    {
        let mut child = std::process::Command::new(std::env::current_exe().unwrap());
        child
            .args([
                "--exact",
                "review_env::budget_001_from_env_overrides_defaults_and_rejects_each_invalid_field",
                "--nocapture",
            ])
            .env("BUDGET_CONFIG_CASE", case);
        for (name, value) in ENV_VALUES {
            child.env_remove(name);
            if case == "all" {
                child.env(name, value);
            }
        }
        if case == "empty-hosts" {
            child.env("PAGE_BUDGET_ALLOWED_THIRD_PARTY_HOSTS", "");
        }
        if case.starts_with("PAGE_BUDGET_") {
            child.env(
                case,
                match case {
                    "PAGE_BUDGET_FINGERPRINT_PATTERN" => "[",
                    "PAGE_BUDGET_ALLOWED_THIRD_PARTY_HOSTS" => "https://invalid/path",
                    _ => "invalid",
                },
            );
        }
        let output = child.output().unwrap();
        assert!(
            output.status.success(),
            "{case}: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[cfg(unix)]
#[test]
fn budget_001_from_env_rejects_non_unicode_values() {
    use std::os::unix::ffi::OsStringExt;
    if std::env::var_os("BUDGET_NONUNICODE_CHILD").is_some() {
        let error = PageBudget::from_env().unwrap_err().to_string();
        assert!(error.contains("PAGE_BUDGET_ASSET_BYTES"), "{error}");
        return;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "review_env::budget_001_from_env_rejects_non_unicode_values",
            "--nocapture",
        ])
        .env("BUDGET_NONUNICODE_CHILD", "1")
        .env(
            "PAGE_BUDGET_ASSET_BYTES",
            std::ffi::OsString::from_vec(vec![255]),
        )
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
