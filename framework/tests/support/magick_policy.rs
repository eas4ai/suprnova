//! What a stand-in `magick` binary answers to the driver's checks of the
//! policy in effect.
//!
//! The `magick` driver checks the ImageMagick policy in effect before the
//! first run of a process (HRD-010). It reads `magick -list policy`, and
//! refuses to run unless the listing shows every denial of the framework's
//! policy in force and the framework's policy file as the last one
//! ImageMagick read. It then runs one MVG probe, an MVG script on stdin read
//! as `mvg:-`, and refuses to run unless ImageMagick's security policy
//! refuses it. A stand-in answers the listing in the shape ImageMagick 7
//! prints (a `Path:` line for each file, then each entry as a `Policy:` line
//! with its `rights:` and `pattern:` fields), and the probe as ImageMagick
//! does under the framework's policy.
//!
//! The driver starts ImageMagick with `HOME` naming its policy directory,
//! which differs per process, and ImageMagick names the framework's file
//! `$HOME/.config/ImageMagick/policy.xml` in the listing. A listing written
//! here names that directory [`POLICY_HOME`], and a stand-in puts its own
//! `HOME` in its place as it answers `-list policy`.
//!
//! Shared across test binaries via `#[path]`.

use std::path::Path;

/// The file a stand-in answers `-list policy` with.
pub const LISTING: &str = "policy-listing";

/// What a listing names in place of the driver's policy directory. A
/// stand-in replaces it with its own `HOME` as it answers `-list policy`.
pub const POLICY_HOME: &str = "__SUPRNOVA_POLICY_HOME__";

/// The framework's policy file as a listing names it: [`POLICY_HOME`]
/// followed by the path ImageMagick reads under `HOME`.
pub const FRAMEWORK_POLICY: &str = "__SUPRNOVA_POLICY_HOME__/.config/ImageMagick/policy.xml";

/// The files a stand-in answers the MVG probe from: what it writes to
/// stdout and to stderr, and its exit code.
pub const PROBE_STDOUT: &str = "mvg-probe.stdout";
pub const PROBE_STDERR: &str = "mvg-probe.stderr";
pub const PROBE_EXIT: &str = "mvg-probe.exit";

/// What ImageMagick 7.1 writes when its policy refuses the MVG probe.
pub const MVG_REFUSAL: &str = "magick: attempt to perform an operation not authorized by the \
                               security policy `MVG' @ error/constitute.c/IsCoderAuthorized/456.\n";

/// One listed entry: its domain, rights and pattern, as ImageMagick prints
/// them (`Coder`, `None`, `MVG`).
pub type Entry<'a> = (&'a str, &'a str, &'a str);

/// A listing of `files`, each a path and its entries, in the order
/// ImageMagick read them, ending with ImageMagick's built-in policy.
pub fn listing(files: &[(&str, &[Entry<'_>])]) -> String {
    let mut listing = String::new();
    for (path, entries) in files {
        listing.push_str(&format!("\nPath: {path}\n"));
        for (domain, rights, pattern) in *entries {
            listing.push_str(&format!(
                "  Policy: {domain}\n    rights: {rights} \n    pattern: {pattern}\n"
            ));
        }
    }
    listing.push_str("\nPath: [built-in]\n  Policy: Undefined\n    rights: None \n");
    listing
}

/// The entries of the framework's own policy, read from
/// `MagickCliDriver::POLICY`, as ImageMagick lists them.
pub fn framework_entries() -> Vec<Entry<'static>> {
    suprnova::MagickCliDriver::POLICY
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("<policy "))
        .map(|line| {
            let parts: Vec<&'static str> = line.split('"').collect();
            let domain = match parts[1] {
                "coder" => "Coder",
                "delegate" => "Delegate",
                "path" => "Path",
                other => panic!("a domain the listing helper does not know: {other}"),
            };
            assert_eq!(parts[3], "none", "the framework's policy only denies");
            (domain, "None", parts[5])
        })
        .collect()
}

/// The listing ImageMagick prints when the framework's policy, read from
/// the driver's `HOME`, is the only file with entries: what the driver's
/// check passes.
pub fn framework_listing() -> String {
    listing(&[(FRAMEWORK_POLICY, &framework_entries())])
}

/// Write, in `dir`, the answers that pass both checks (the framework's
/// listing, and ImageMagick's refusal of the probe), and return the first
/// lines of a stand-in script that answer each check from them and do
/// nothing else for it. The listing is answered with [`POLICY_HOME`]
/// replaced by the stand-in's `HOME`. A test changes an answer by writing
/// its file again.
pub fn answer_policy_checks(dir: &Path) -> String {
    let listing = dir.join(LISTING);
    let (stdout, stderr, exit) = (
        dir.join(PROBE_STDOUT),
        dir.join(PROBE_STDERR),
        dir.join(PROBE_EXIT),
    );
    std::fs::write(&listing, framework_listing()).expect("write the listing");
    std::fs::write(&stdout, b"").expect("write the probe's stdout");
    std::fs::write(&stderr, MVG_REFUSAL).expect("write the probe's stderr");
    std::fs::write(&exit, b"1").expect("write the probe's exit code");
    format!(
        "if [ \"$1\" = -list ] && [ \"$2\" = policy ]; then\n\
         exec sed \"s|{POLICY_HOME}|$HOME|g\" '{listing}'\n\
         fi\n\
         for arg; do\n\
         if [ \"$arg\" = mvg:- ]; then\n\
         cat > /dev/null\n\
         cat '{stdout}'\n\
         cat '{stderr}' >&2\n\
         exit \"$(cat '{exit}')\"\n\
         fi\n\
         done\n",
        listing = listing.display(),
        stdout = stdout.display(),
        stderr = stderr.display(),
        exit = exit.display(),
    )
}
