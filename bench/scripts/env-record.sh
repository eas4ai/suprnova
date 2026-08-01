#!/usr/bin/env bash
#
# Capture the environment stanza that every result set must carry.
#
# A benchmark number without its environment is an anecdote. This records
# what was measured on, what was measured, and - because the SUT here is a
# shared Coolify host running live products - what else was resident at the
# time. That last part is what stops a number being waved away later.

set -euo pipefail

# Runs both from a checkout and from a bare copy dropped on the SUT host,
# where there is no repo to interrogate. The code-under-test stanza is
# skipped in that case rather than the script failing.
REPO_ROOT="$(git rev-parse --show-toplevel 2>/dev/null || true)"
DEFAULT_DIR="${REPO_ROOT:-$PWD}/bench/results/$(date -u +%Y%m%dT%H%M%SZ)-env"
OUT="${1:-$DEFAULT_DIR/env.txt}"
mkdir -p "$(dirname "$OUT")"

{
    echo "# Environment record"
    echo "captured_utc: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
    echo "captured_on:  $(hostname)"
    echo

    echo "## Code under test"
    if [[ -n "$REPO_ROOT" ]]; then
        echo "commit:      $(git -C "$REPO_ROOT" rev-parse HEAD)"
        echo "describe:    $(git -C "$REPO_ROOT" describe --tags --always --dirty 2>/dev/null || echo unknown)"
        echo "tree_dirty:  $([[ -n "$(git -C "$REPO_ROOT" status --porcelain)" ]] && echo yes || echo no)"
    else
        echo "commit:      (no checkout here - record the image tag instead)"
        echo "image_tag:   ${BENCH_IMAGE:-unset}"
    fi
    echo "rustc:       $(rustc --version 2>/dev/null || echo 'not installed')"
    echo "profile:     ${BENCH_PROFILE:-release}"
    echo

    echo "## Machine"
    echo "kernel:      $(uname -sr)"
    if command -v lscpu >/dev/null; then
        lscpu | grep -E "^Model name|^CPU\(s\)|^Thread\(s\) per core|^Core\(s\) per socket|^Socket\(s\)|^NUMA node\(s\)|^CPU max MHz" \
            | sed 's/^/cpu_/'
    fi
    echo "memory_gb:   $(free -g 2>/dev/null | awk '/^Mem:/{print $2}')"
    echo "disk_root:   $(df -h / | awk 'NR==2{print $2" total, "$4" free, "$5" used"}')"
    echo "loadavg:     $(cut -d' ' -f1-3 /proc/loadavg 2>/dev/null)"
    echo

    # The point of this section: this host is not dedicated to the
    # benchmark. Anything measured here shares the box with live services,
    # and ClickHouse in particular is bursty. Record the tenancy so a
    # surprising number can be checked against it rather than argued about.
    echo "## Resident workload at capture time"
    if command -v docker >/dev/null && docker info >/dev/null 2>&1; then
        echo "docker:      $(docker --version)"
        echo "containers_running: $(docker ps -q | wc -l)"
        docker ps --format '  {{.Names}}\t{{.Image}}\t{{.Status}}' 2>/dev/null || true
    else
        echo "docker:      not available from here"
    fi
    echo

    echo "## Load generator"
    for tool in vegeta oha iperf3; do
        if command -v "$tool" >/dev/null; then
            printf '%-10s %s\n' "$tool:" "$("$tool" --version 2>&1 | head -1)"
        else
            printf '%-10s %s\n' "$tool:" "not installed"
        fi
    done
} | tee "$OUT"

echo
echo "==> wrote $OUT"
