use std::sync::Once;

use suprnova::crypto::{Crypt, EncryptionKey};
use suprnova::live::testing::{
    inspect_deterministic_upload_handle_for_test, inspect_upload_mount_authority_for_test,
    prepare_live_router_for_test, register_live_mount_for_test,
    resolve_upload_mount_authority_for_test, select_upload_mount_for_test,
};
use suprnova::live::{LiveComponent, LiveRegistry, live};
use suprnova::{App, Router};

#[derive(LiveComponent)]
#[live(
    name = "tests.upload-security-component",
    view = "live/tests/boot-component.html"
)]
pub struct UploadSecurityComponent {
    count: u64,
}

#[test]
fn deterministic_upload_handles_are_keyed_mount_bound_and_rotation_aware() {
    init_runtime_dependencies();

    let mut router = Router::new();
    register_live_mount_for_test::<UploadSecurityComponent>(&mut router, "/uploads-a", "first")
        .expect("register first upload mount");
    register_live_mount_for_test::<UploadSecurityComponent>(&mut router, "/uploads-b", "second")
        .expect("register second upload mount");
    let runtime = prepare_live_router_for_test(&router).expect("finalize upload mount catalog");
    let first = inspect_upload_mount_authority_for_test(
        &runtime,
        "tests.upload-security-component",
        "first",
        "first",
        Some(b"session-a"),
        Some(b"principal-a"),
        Some(b"tenant-a"),
    )
    .expect("derive first trusted mount authority");
    let second = inspect_upload_mount_authority_for_test(
        &runtime,
        "tests.upload-security-component",
        "second",
        "second",
        Some(b"session-a"),
        Some(b"principal-a"),
        Some(b"tenant-a"),
    )
    .expect("derive second trusted mount authority");
    let active_key = [0x11; 32];
    let previous_key = [0x22; 32];

    let first_issue = inspect_deterministic_upload_handle_for_test(
        &first,
        "avatar",
        "browser-chosen-idempotency",
        &active_key,
        &[],
        None,
        None,
    )
    .expect("derive active upload handle");
    let exact_replay = inspect_deterministic_upload_handle_for_test(
        &first,
        "avatar",
        "browser-chosen-idempotency",
        &active_key,
        &[],
        None,
        None,
    )
    .expect("derive exact replay upload handle");
    assert!(first_issue.same_current(&exact_replay));
    assert!(first_issue.is_uuid_v4());

    for separated in [
        inspect_deterministic_upload_handle_for_test(
            &second,
            "avatar",
            "browser-chosen-idempotency",
            &active_key,
            &[],
            None,
            None,
        ),
        inspect_deterministic_upload_handle_for_test(
            &first,
            "attachment",
            "browser-chosen-idempotency",
            &active_key,
            &[],
            None,
            None,
        ),
        inspect_deterministic_upload_handle_for_test(
            &first,
            "avatar",
            "browser-chosen-idempotency",
            &active_key,
            &[],
            Some("stale-build"),
            None,
        ),
        inspect_deterministic_upload_handle_for_test(
            &first,
            "avatar",
            "browser-chosen-idempotency",
            &active_key,
            &[],
            None,
            Some(&[0x5a; 32]),
        ),
        inspect_deterministic_upload_handle_for_test(
            &first,
            "avatar",
            "attacker-chosen-neighbor",
            &active_key,
            &[],
            None,
            None,
        ),
    ] {
        let separated = separated.expect("derive separated upload handle");
        assert!(
            !first_issue.same_current(&separated),
            "mount, field, build, contract, and idempotency inputs must be authority-bound"
        );
    }

    let old_issue = inspect_deterministic_upload_handle_for_test(
        &first,
        "avatar",
        "browser-chosen-idempotency",
        &previous_key,
        &[],
        None,
        None,
    )
    .expect("derive pre-rotation upload handle");
    let rotated = inspect_deterministic_upload_handle_for_test(
        &first,
        "avatar",
        "browser-chosen-idempotency",
        &active_key,
        &[&previous_key],
        None,
        None,
    )
    .expect("derive post-rotation upload handle candidates");
    assert!(!rotated.same_current(&old_issue));
    assert!(rotated.accepts(&old_issue));

    let diagnostic = format!("{rotated:?}");
    assert!(!diagnostic.contains("browser-chosen-idempotency"));
    assert!(!diagnostic.contains("uploads-a"));
    assert!(!diagnostic.contains("avatar"));
}

#[live]
impl UploadSecurityComponent {}

fn init_runtime_dependencies() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        Crypt::init(EncryptionKey::generate());
        App::singleton(
            LiveRegistry::builder()
                .register::<UploadSecurityComponent>()
                .expect("register upload security component")
                .build(),
        );
    });
}

#[test]
fn upload_mount_selection_is_finalized_unique_and_server_owned() {
    init_runtime_dependencies();

    let mut unique = Router::new();
    register_live_mount_for_test::<UploadSecurityComponent>(&mut unique, "/uploads-a", "primary")
        .expect("register unique upload mount");
    let runtime = prepare_live_router_for_test(&unique).expect("finalize unique mount catalog");
    select_upload_mount_for_test(
        &runtime,
        "tests.upload-security-component",
        "primary",
        "primary",
    )
    .expect("select exact finalized mount");
    assert!(
        select_upload_mount_for_test(&runtime, "tests.foreign-component", "primary", "primary",)
            .is_err(),
        "a browser component value cannot supply trusted catalog facts"
    );
    assert!(
        select_upload_mount_for_test(
            &runtime,
            "tests.upload-security-component",
            "primary",
            "foreign-document",
        )
        .is_err(),
        "a browser document key may select only a server-declared candidate"
    );

    let mut ambiguous = Router::new();
    register_live_mount_for_test::<UploadSecurityComponent>(
        &mut ambiguous,
        "/uploads-b",
        "primary",
    )
    .expect("register first ambiguous candidate");
    register_live_mount_for_test::<UploadSecurityComponent>(
        &mut ambiguous,
        "/uploads-c",
        "primary",
    )
    .expect("register second ambiguous candidate");
    let runtime = prepare_live_router_for_test(&ambiguous).expect("finalize ambiguous catalog");
    assert!(
        select_upload_mount_for_test(
            &runtime,
            "tests.upload-security-component",
            "primary",
            "primary",
        )
        .is_err(),
        "multiple finalized route candidates must fail closed"
    );

    let mut collision = Router::new();
    register_live_mount_for_test::<UploadSecurityComponent>(
        &mut collision,
        "/uploads-d",
        "primary",
    )
    .expect("register first colliding mount");
    register_live_mount_for_test::<UploadSecurityComponent>(
        &mut collision,
        "/uploads-d",
        "primary",
    )
    .expect("record duplicate for startup validation");
    assert!(
        prepare_live_router_for_test(&collision).is_err(),
        "duplicate trusted route/slot catalog ownership must fail at startup"
    );
}

#[test]
fn upload_authority_is_bound_to_one_finalized_mount_and_current_host_scope() {
    init_runtime_dependencies();

    let mut router = Router::new();
    register_live_mount_for_test::<UploadSecurityComponent>(&mut router, "/uploads-a", "first")
        .expect("register first upload mount");
    register_live_mount_for_test::<UploadSecurityComponent>(&mut router, "/uploads-b", "second")
        .expect("register second upload mount");
    let runtime = prepare_live_router_for_test(&router).expect("finalize upload mount catalog");

    let first = inspect_upload_mount_authority_for_test(
        &runtime,
        "tests.upload-security-component",
        "first",
        "first",
        Some(b"session-a"),
        Some(b"principal-a"),
        Some(b"tenant-a"),
    )
    .expect("derive first trusted mount authority");
    let second = inspect_upload_mount_authority_for_test(
        &runtime,
        "tests.upload-security-component",
        "second",
        "second",
        Some(b"session-a"),
        Some(b"principal-a"),
        Some(b"tenant-a"),
    )
    .expect("derive second trusted mount authority");
    assert!(
        !first.same_scope(&second),
        "same-component mounts on different finalized route/slot facts cannot share authority"
    );

    let resolved = resolve_upload_mount_authority_for_test(
        &runtime,
        "tests.upload-security-component",
        &first,
        Some(b"session-a"),
        Some(b"principal-a"),
        Some(b"tenant-a"),
    )
    .expect("resolve one exact authority among same-component mounts");
    assert!(resolved.matches("first", "first"));
    assert!(
        !resolved.matches("second", "second"),
        "a browser cannot substitute another same-component mount after grant issuance"
    );

    for changed in [
        resolve_upload_mount_authority_for_test(
            &runtime,
            "tests.upload-security-component",
            &first,
            Some(b"session-b"),
            Some(b"principal-a"),
            Some(b"tenant-a"),
        ),
        resolve_upload_mount_authority_for_test(
            &runtime,
            "tests.upload-security-component",
            &first,
            Some(b"session-a"),
            Some(b"principal-a"),
            Some(b"tenant-b"),
        ),
    ] {
        assert!(
            changed.is_err(),
            "current session or tenant drift must reject before upload authority is used"
        );
    }

    assert!(!first.matches_build("stale-build"));
    assert!(!first.matches_contract(&[0x5a; 32]));
    for diagnostic in [format!("{first:?}"), format!("{resolved:?}")] {
        assert!(!diagnostic.contains("uploads-a"));
        assert!(!diagnostic.contains("first"));
        assert!(!diagnostic.contains("tests.upload-security-component"));
    }
}
