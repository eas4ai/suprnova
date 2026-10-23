#![no_main]

mod support;

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if data.len() > 4_096 {
        return;
    }
    let fragment = String::from_utf8_lossy(data);
    let source = format!("<button live:{fragment}></button>");
    let Some(report) = support::check_template_source(&source) else {
        return;
    };
    assert!(report.diagnostics().len() <= 64);
});
