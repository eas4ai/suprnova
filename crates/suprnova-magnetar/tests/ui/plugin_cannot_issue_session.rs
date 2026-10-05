use magnetar::sessions::{SessionGrant, SessionIssuer};

fn main() {
    // Naming one field keeps this diagnostic identical from Rust 1.94 (the
    // MSRV) to the pinned toolchain. With an empty literal, 1.94 and 1.95
    // add a "private fields ... that were not provided" note that 1.97 and
    // later omit, and trybuild compares stderr byte for byte.
    let _ = SessionGrant {
        session_id: String::new(),
    };
    let _ = SessionIssuer;
}
