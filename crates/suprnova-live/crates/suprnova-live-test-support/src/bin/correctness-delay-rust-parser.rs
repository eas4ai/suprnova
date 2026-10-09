//! Parse-only helper for the Iteration 004 correctness-delay gate.

use std::io::{self, Read as _};

use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
struct Candidate {
    file_path: String,
    source: String,
}

#[derive(Serialize)]
struct ParseFailure {
    file_path: String,
    kind: &'static str,
    line: usize,
}

fn main() {
    let mut encoded = String::new();
    io::stdin()
        .read_to_string(&mut encoded)
        .expect("read parser candidates");
    let candidates: Vec<Candidate> =
        serde_json::from_str(&encoded).expect("decode parser candidates");
    let failures = candidates
        .into_iter()
        .filter_map(|candidate| {
            syn::parse_file(&candidate.source)
                .err()
                .map(|_| ParseFailure {
                    file_path: candidate.file_path,
                    kind: "parse-error",
                    line: 1,
                })
        })
        .collect::<Vec<_>>();
    serde_json::to_writer(io::stdout(), &failures).expect("encode parser failures");
}
