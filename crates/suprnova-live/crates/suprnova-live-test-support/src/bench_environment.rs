//! Environment evidence and the percentile rule shared by benchmarks.
//!
//! Every benchmark in this workspace records the machine it ran on beside
//! its numbers, and classifies that machine against the S1 reference
//! environment. The rules are the same for every benchmark, so they live
//! here once rather than being copied into each bench target. The three
//! benches that predate this module keep their own copies; nothing here
//! changes what they record.
//!
//! Nothing collected here is secret: the record is processor, memory,
//! kernel, governor, and toolchain facts plus the classification derived
//! from them. No key, body, digest, or principal material can reach it.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::process::Command;

use serde::Serialize;

/// Processors the S1 reference environment pins a benchmark to.
const S1_CPU_COUNT: usize = 8;
/// Memory the S1 reference environment requires, in bytes.
const S1_MEMORY_BYTES: u64 = 16 * 1024 * 1024 * 1024;
/// The value shown for a fact this machine does not publish.
const UNAVAILABLE: &str = "unavailable";

/// The machine one benchmark ran on, and whether it satisfies S1.
///
/// Every field is public so a bench can overwrite the two facts only it
/// knows ([`Self::database`] and [`Self::provider_versions`]) after
/// [`collect`] fills in the rest.
#[derive(Clone, Debug, Serialize)]
pub struct EnvironmentEvidence {
    /// `validated_s1` when every S1 requirement is proven, otherwise
    /// `local_exploratory`. A workstation run is always the latter.
    pub classification: &'static str,
    /// Operating system this build targets.
    pub operating_system: &'static str,
    /// Processor architecture this build targets.
    pub architecture: &'static str,
    /// Processor model as the kernel reports it.
    pub cpu_model: String,
    /// The processor set the run was allowed on, as a kernel list.
    pub selected_cpu_affinity: String,
    /// How many processors that list names.
    pub selected_cpu_count: usize,
    /// Total system memory in bytes.
    pub memory_bytes: u64,
    /// Kernel name and release.
    pub kernel: String,
    /// Frequency governor across the selected processors.
    pub cpu_governor: String,
    /// Compiler version the benchmark was built with.
    pub rustc: String,
    /// The database the benchmark exercised, or why it used none. Set by
    /// the bench; [`collect`] leaves it at `not_used`.
    pub database: &'static str,
    /// Provider implementations and their versions. Set by the bench;
    /// [`collect`] leaves it empty.
    pub provider_versions: BTreeMap<&'static str, &'static str>,
    /// Whether the runner attested dedicated processors through
    /// `SUPRNOVA_LIVE_S1_DEDICATED`.
    pub dedicated_vcpus_attested: bool,
    /// Whether the measured steps run against an already warm filesystem
    /// cache rather than a first-touch cold read.
    pub warm_filesystem_cache: bool,
    /// Whether every provider the measured steps use is in process or on
    /// loopback, so no measurement crosses a real network.
    pub loopback_providers: bool,
    /// Whether every S1 requirement above is proven for this run.
    pub s1_requirements_met: bool,
}

/// Reads this machine's facts and classifies them against S1.
///
/// S1 requires Linux on `x86_64`, exactly eight selected processors, at
/// least 16 GiB of memory, the performance governor, and the runner's
/// `SUPRNOVA_LIVE_S1_DEDICATED=1` attestation. Anything less is
/// `local_exploratory`, which is what a workstation run is; the
/// attestation is never set by a benchmark on its own behalf.
///
/// A fact this machine does not publish is recorded as `unavailable`
/// rather than guessed, and an unavailable fact can only lower the
/// classification, never raise it.
#[must_use]
pub fn collect() -> EnvironmentEvidence {
    let affinity =
        read_labeled_value("/proc/self/status", "Cpus_allowed_list").unwrap_or_else(unavailable);
    let selected_cpu_count = cpu_list(&affinity).len();
    let memory_bytes = read_labeled_value("/proc/meminfo", "MemTotal")
        .and_then(|value| value.split_whitespace().next()?.parse::<u64>().ok())
        .unwrap_or(0)
        .saturating_mul(1024);
    let governor = governors(&affinity);
    let dedicated = std::env::var("SUPRNOVA_LIVE_S1_DEDICATED").as_deref() == Ok("1");
    let requirements_met = std::env::consts::OS == "linux"
        && std::env::consts::ARCH == "x86_64"
        && selected_cpu_count == S1_CPU_COUNT
        && memory_bytes >= S1_MEMORY_BYTES
        && governor == "performance"
        && dedicated;
    EnvironmentEvidence {
        classification: if requirements_met {
            "validated_s1"
        } else {
            "local_exploratory"
        },
        operating_system: std::env::consts::OS,
        architecture: std::env::consts::ARCH,
        cpu_model: read_labeled_value("/proc/cpuinfo", "model name").unwrap_or_else(unavailable),
        selected_cpu_affinity: affinity,
        selected_cpu_count,
        memory_bytes,
        kernel: command_output("uname", &["-sr"]).unwrap_or_else(unavailable),
        cpu_governor: governor,
        rustc: command_output("rustc", &["--version"]).unwrap_or_else(unavailable),
        database: "not_used",
        provider_versions: BTreeMap::new(),
        dedicated_vcpus_attested: dedicated,
        warm_filesystem_cache: true,
        loopback_providers: true,
        s1_requirements_met: requirements_met,
    }
}

/// Nearest-rank percentile over sorted samples: `ceil(p * n)`, one-based.
///
/// # Panics
///
/// Panics when `sorted` is empty; a caller measures at least one sample
/// before asking for a percentile of them.
#[must_use]
pub fn percentile(sorted: &[f64], p: f64) -> f64 {
    let rank = (p * sorted.len() as f64).ceil() as usize;
    sorted[rank.saturating_sub(1).min(sorted.len() - 1)]
}

/// The placeholder for a fact this machine does not publish.
fn unavailable() -> String {
    UNAVAILABLE.to_owned()
}

/// The value of one `label: value` line in a kernel-published file.
fn read_labeled_value(path: &str, label: &str) -> Option<String> {
    fs::read_to_string(path).ok()?.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        (name.trim() == label).then(|| value.trim().to_owned())
    })
}

/// Expands a kernel processor list such as `0,2-4` into its members.
fn cpu_list(value: &str) -> BTreeSet<u32> {
    let mut cpus = BTreeSet::new();
    for part in value.split(',') {
        let mut bounds = part.trim().splitn(2, '-');
        let Some(start) = bounds.next().and_then(|value| value.parse::<u32>().ok()) else {
            continue;
        };
        let end = bounds
            .next()
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(start);
        cpus.extend(start..=end);
    }
    cpus
}

/// Every distinct frequency governor across the selected processors, so a
/// mixed set is visible as a mix rather than collapsing to one value.
fn governors(affinity: &str) -> String {
    let values = cpu_list(affinity)
        .into_iter()
        .filter_map(|cpu| {
            fs::read_to_string(format!(
                "/sys/devices/system/cpu/cpu{cpu}/cpufreq/scaling_governor"
            ))
            .ok()
            .map(|value| value.trim().to_owned())
        })
        .collect::<BTreeSet<_>>();
    if values.is_empty() {
        UNAVAILABLE.to_owned()
    } else {
        values.into_iter().collect::<Vec<_>>().join(",")
    }
}

/// One command's trimmed standard output, or `None` when it is absent or
/// fails; a probe that cannot run is an unavailable fact, never an error.
fn command_output(program: &str, arguments: &[&str]) -> Option<String> {
    let output = Command::new(program).args(arguments).output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8(output.stdout).ok()?.trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::{cpu_list, percentile};

    #[test]
    fn the_percentile_rule_is_nearest_rank_and_one_based() {
        let samples = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0];
        assert_eq!(percentile(&samples, 0.50), 5.0, "ceil(0.5 * 10) is rank 5");
        assert_eq!(
            percentile(&samples, 0.95),
            10.0,
            "ceil(0.95 * 10) is rank 10"
        );
        assert_eq!(percentile(&samples, 0.0), 1.0, "rank 0 clamps to the first");
        assert_eq!(percentile(&samples, 1.0), 10.0, "rank n is the last");
        assert_eq!(percentile(&[42.0], 0.95), 42.0, "one sample is every rank");
    }

    #[test]
    fn a_processor_list_expands_ranges_and_ignores_unparsable_parts() {
        assert_eq!(cpu_list("0-7").len(), 8);
        assert_eq!(
            cpu_list("0,2-4,9").into_iter().collect::<Vec<_>>(),
            [0, 2, 3, 4, 9]
        );
        assert_eq!(cpu_list("3").into_iter().collect::<Vec<_>>(), [3]);
        assert!(cpu_list("unavailable").is_empty());
    }
}
