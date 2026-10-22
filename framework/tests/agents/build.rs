//! The rules a tool's marking must follow fail the build when broken
//! (AGENT-001, AGENT-002). Only a real build proves a build failure, so each
//! case compiles under `trybuild` and its error is compared with the
//! recorded one.

#[test]
fn agent_001_invalid_tool_markings_fail_to_build() {
    trybuild::TestCases::new().compile_fail("tests/agents/compile_fail/agent_001_*.rs");
}

#[test]
fn agent_002_undeclared_scope_fails_to_build() {
    trybuild::TestCases::new().compile_fail("tests/agents/compile_fail/agent_002_*.rs");
}

/// The templates the Live cases' components render, where the build
/// `trybuild` runs looks for them: its own crate's `templates/`.
fn install_live_templates() {
    let target = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"))
        .parent()
        .expect("CARGO_TARGET_TMPDIR sits inside the target directory")
        .to_path_buf();
    let directory = target.join("tests/trybuild/suprnova/templates/live/agents");
    std::fs::create_dir_all(&directory).expect("create the trybuild template directory");
    std::fs::write(directory.join("tool.html"), "<p>{{ count }}</p>\n")
        .expect("write the trybuild template");
}

#[test]
fn agent_001_live_tool_markings_fail_to_build() {
    install_live_templates();
    trybuild::TestCases::new().compile_fail("tests/agents/compile_fail/live/agent_001_*.rs");
}

#[test]
fn agent_002_live_tool_with_undeclared_scope_fails_to_build() {
    install_live_templates();
    trybuild::TestCases::new().compile_fail("tests/agents/compile_fail/live/agent_002_*.rs");
}

#[test]
fn agent_001_budget_warnings_allow_a_64_character_name_to_build() {
    install_live_templates();
    trybuild::TestCases::new().pass("tests/agents/compile_pass/agent_001_*.rs");
}

/// The unmangled name proves `no_mangle` applied; without it the name is
/// v0-mangled. The linker rejects two global definitions of one name during
/// resolution. The executable localizes the survivor afterwards, so this
/// test does not check its final binding. A Windows executable carries no
/// symbol table to read, so the test runs only where the executable records
/// its symbols; the MSVC linker refuses two definitions of one name all the
/// same.
#[test]
#[cfg(not(windows))]
fn agent_001_tool_has_an_unmangled_one_byte_data_symbol() {
    use object::{Object, ObjectSymbol, SymbolKind};

    // Keep the address live through section collection without reading the byte.
    std::hint::black_box(&crate::fixture::__suprnova_tool_echo);
    let executable = std::env::current_exe().expect("locate the agents test binary");
    let bytes = std::fs::read(executable).expect("read the agents test binary");
    let object = object::File::parse(bytes.as_slice()).expect("parse the agents test binary");
    let marker = object
        .symbols()
        .find(|symbol| {
            matches!(
                symbol.name(),
                Ok("__suprnova_tool_echo" | "___suprnova_tool_echo")
            )
        })
        .expect("echo exports its unmangled marker");
    assert!(marker.is_definition(), "the marker must define the symbol");
    assert!(!marker.is_weak(), "duplicate markers must be a link error");
    assert_eq!(marker.kind(), SymbolKind::Data);
    #[cfg(not(target_os = "macos"))]
    assert_eq!(marker.size(), 1);
    // Mach-O records no symbol size, so check the generated static's type.
    #[cfg(target_os = "macos")]
    assert_eq!(
        std::mem::size_of_val(&crate::fixture::__suprnova_tool_echo),
        1
    );
}
