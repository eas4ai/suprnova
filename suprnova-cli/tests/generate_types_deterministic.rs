//! `suprnova generate-types` writes a **checked-in** file, so its output
//! has to be a pure function of the source — same input, same bytes.
//!
//! It was not. The topological sort seeded its work queue by iterating a
//! `HashMap`, and Rust randomises hash iteration order per process, so
//! consecutive runs emitted the same interfaces in a different order.
//! Every run produced a diff. A generated artifact that churns for no
//! reason is one people stop regenerating, and then it silently stops
//! matching the Rust it claims to describe.
//!
//! Testing this in a single process cannot work by re-running the same
//! input — within one process the hash seed is fixed, so the order is
//! stable however wrong the algorithm is. What distinguishes a
//! deterministic sort from an accidental one is *order independence*:
//! permuting the declarations must not permute the output.

use suprnova_cli::commands::generate_types::{ScanInput, generate_types_string};

const A: &str = r#"
#[derive(suprnova::InertiaProps)]
pub struct Alpha { pub id: i64 }
"#;

const B: &str = r#"
#[derive(suprnova::InertiaProps)]
pub struct Beta { pub name: String }
"#;

const C: &str = r#"
#[derive(suprnova::InertiaProps)]
pub struct Gamma { pub flag: bool }
"#;

/// Structs with no dependencies between them: nothing constrains their
/// relative order except the sort itself, which is exactly the case the
/// hash seed used to decide.
#[test]
fn independent_structs_emit_in_a_stable_order_regardless_of_declaration_order() {
    let orders = [
        format!("{A}{B}{C}"),
        format!("{C}{B}{A}"),
        format!("{B}{A}{C}"),
        format!("{C}{A}{B}"),
    ];

    // `ScanInput::Source` wants `&'static str`; the permutations are built
    // at runtime. Leaking four short strings in a test process that is
    // about to exit is the cheapest way across that gap.
    let outputs: Vec<String> = orders
        .into_iter()
        .map(|src| generate_types_string(ScanInput::Source(Box::leak(src.into_boxed_str()))))
        .collect();

    for (i, out) in outputs.iter().enumerate().skip(1) {
        assert_eq!(
            &outputs[0], out,
            "permutation {i} of the same three structs produced different output:\n\
             --- first ---\n{}\n--- permutation {i} ---\n{out}",
            outputs[0]
        );
    }

    // Sanity: all three actually made it into the output, so the
    // assertion above is not comparing two empty strings.
    for name in ["Alpha", "Beta", "Gamma"] {
        assert!(
            outputs[0].contains(&format!("interface {name}")),
            "{name} missing from output:\n{}",
            outputs[0]
        );
    }
}

/// A referenced struct and its referrer must come out in the same
/// relative order however they were declared.
///
/// Note which order that is: the generator emits the *referrer* first.
/// That is not a bug to fix — a TypeScript interface may name another
/// declared later in the same file, so declaration order means nothing to
/// the consumer. It is asserted here only so the choice stays a choice
/// rather than drifting silently.
#[test]
fn a_referenced_struct_keeps_a_stable_position_relative_to_its_referrer() {
    const WRAPPER_FIRST: &str = r#"
#[derive(suprnova::InertiaProps)]
pub struct Wrapper { pub inner: Inner }

#[derive(suprnova::InertiaProps)]
pub struct Inner { pub id: i64 }
"#;
    const INNER_FIRST: &str = r#"
#[derive(suprnova::InertiaProps)]
pub struct Inner { pub id: i64 }

#[derive(suprnova::InertiaProps)]
pub struct Wrapper { pub inner: Inner }
"#;

    let a = generate_types_string(ScanInput::Source(WRAPPER_FIRST));
    let b = generate_types_string(ScanInput::Source(INNER_FIRST));
    assert_eq!(
        a, b,
        "declaration order must not change the generated file:\n--- a ---\n{a}\n--- b ---\n{b}"
    );

    let inner = a.find("interface Inner").expect("Inner emitted");
    let wrapper = a.find("interface Wrapper").expect("Wrapper emitted");
    assert!(
        wrapper < inner,
        "the referrer is emitted first; see the note on `topological_sort`:\n{a}"
    );
}
