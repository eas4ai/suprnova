#!/usr/bin/env bash
#
# Thermal guard. Source this; it defines helpers, it does not run anything.
#
# # Why a benchmark needs one
#
# Two reasons, and the second is the one that bites quietly.
#
# Safety: a sustained campaign holds every core near boost for hours. If
# the package reaches Tjmax the host may cut power, and a benchmark that
# kills the machine it runs on has failed at something more basic than
# measurement.
#
# Validity: long before anything is at risk, a hot package downclocks.
# A run whose cores sat at 5.2 GHz for the first ten minutes and 4.1 GHz
# for the last fifty did not measure one system — it measured two, and
# averaged them. That is the same class of silent corruption as a
# saturated load generator: the number looks fine and means nothing.
#
# # The rule this file encodes
#
#   Cool BETWEEN runs. Back off DURING them. Record every backoff.
#
# Backing off rather than aborting keeps a multi-hour campaign alive: one
# warm afternoon should not cost the whole night's work. The cost is that
# a run which backed off is no longer a single measurement at a single
# load, so it cannot be quietly averaged in with runs that did not. Every
# backoff is timestamped and the verdict marks the run load-limited, so
# the choice is between "re-run this one" and "quote it with a caveat" —
# never between a clean number and a corrupted one that looks identical.
#
# Mechanically, load cannot be lowered inside a running `vegeta attack` —
# the rate is fixed for the life of the process. So long runs are split
# into segments and the rate is chosen per segment. That segmentation
# pays for itself anyway: it is also what lets an interrupted campaign
# resume without losing the measurements before the interruption.
#
# Cooling between runs has a second benefit beyond safety. A run starting
# from 42 °C and one starting from 85 °C are not the same experiment —
# the second begins already down-clocked. Equalising the starting
# temperature makes consecutive runs comparable, which matters most for
# the A/B against Laravel where the two stacks run back to back.

# Backoff trips at 88 so that the *peak* lands at about 90 rather than
# starting to react there — by the time a sample reads 90 the package has
# already been climbing for a sampling interval. Zen 5 Tjmax is 95.
: "${THERMAL_BACKOFF_C:=88}"
: "${THERMAL_WARN_C:=80}"    # record, do not act
: "${THERMAL_COOL_C:=60}"    # start every run at or below this
: "${THERMAL_COOL_TIMEOUT:=600}"
# Each backoff drops the next segment's offered load by this much, to a
# floor. The floor exists so a runaway sensor cannot silently walk a
# measurement down to nothing and still call it a run.
: "${THERMAL_BACKOFF_STEP_PCT:=15}"
: "${THERMAL_BACKOFF_FLOOR_PCT:=50}"
# Clock drift only counts as thermal throttling if the package actually
# got hot. Below this, a falling mean clock is the governor parking idle
# cores, not heat — an idle box reads a 30%+ "drop" between two samples
# taken seconds apart. Reporting that as degradation trains the reader to
# ignore the one warning that matters.
: "${THERMAL_DRIFT_MIN_C:=70}"

# The hwmon index is not stable across reboots — this box has renumbered
# twice today — so the sensor is discovered by name every time rather
# than hardcoded to whatever it was when this was written.
thermal_sensor_path() {
    local h name
    for h in /sys/class/hwmon/hwmon*; do
        [[ -r "$h/name" ]] || continue
        name="$(<"$h/name")"
        if [[ "$name" == "k10temp" && -r "$h/temp1_input" ]]; then
            printf '%s\n' "$h/temp1_input"
            return 0
        fi
    done
    # Fall back to the ACPI thermal zone. Coarser and slower to react
    # than the on-die sensor, but present on machines without k10temp.
    if [[ -r /sys/class/thermal/thermal_zone0/temp ]]; then
        printf '%s\n' /sys/class/thermal/thermal_zone0/temp
        return 0
    fi
    return 1
}

THERMAL_SENSOR="$(thermal_sensor_path || true)"

# Integer degrees C. Returns empty when no sensor was found, and callers
# treat that as "unknown" rather than "cool" — a guard that silently
# passes when it cannot read anything is worse than no guard.
thermal_temp_c() {
    [[ -n "$THERMAL_SENSOR" && -r "$THERMAL_SENSOR" ]] || return 1
    local milli
    milli="$(<"$THERMAL_SENSOR")"
    printf '%s\n' $(( milli / 1000 ))
}

# Mean clock across all cores, in MHz. This is the throttling signal:
# temperature explains why, but the clock is what actually changed the
# measurement.
thermal_mhz_mean() {
    awk '/^cpu MHz/ { s += $4; n++ } END { if (n) printf "%d\n", s / n; else print 0 }' \
        /proc/cpuinfo
}

thermal_available() { [[ -n "$THERMAL_SENSOR" ]]; }

# Block until the package is at or below THERMAL_COOL_C. Called before
# every run so each one starts from the same thermal state.
thermal_wait_cool() {
    thermal_available || {
        echo "    thermal: no sensor found — cannot equalise starting temperature" >&2
        return 0
    }
    local waited=0 t
    t="$(thermal_temp_c)"
    if (( t <= THERMAL_COOL_C )); then
        printf '    thermal: %d°C, at or below %d°C floor\n' "$t" "$THERMAL_COOL_C"
        return 0
    fi
    printf '    thermal: %d°C, waiting for %d°C' "$t" "$THERMAL_COOL_C"
    while (( waited < THERMAL_COOL_TIMEOUT )); do
        sleep 10
        waited=$(( waited + 10 ))
        t="$(thermal_temp_c)"
        printf '.'
        if (( t <= THERMAL_COOL_C )); then
            printf ' reached %d°C after %ds\n' "$t" "$waited"
            return 0
        fi
    done
    printf '\n    thermal: still %d°C after %ds — proceeding, and the run is\n' \
        "$t" "$waited"
    printf '    marked as having started hot rather than silently compared\n'
    printf '    against runs that started cold\n'
    return 1
}

# Sample temperature and mean clock into a CSV for the duration of a run.
# Touches <out>.backoff when the package crosses THERMAL_BACKOFF_C, and
# keeps sampling — the run continues, the next segment runs cooler.
# Echoes the sampler PID.
thermal_watch() {
    local out="$1" interval="${2:-5}"
    printf 'ts_unix,temp_c,mhz_mean\n' >"$out"
    (
        while :; do
            local t m
            t="$(thermal_temp_c || echo '')"
            m="$(thermal_mhz_mean)"
            printf '%s,%s,%s\n' "$(date +%s)" "$t" "$m" >>"$out"
            if [[ -n "$t" ]] && (( t >= THERMAL_BACKOFF_C )); then
                printf '%s %s\n' "$(date +%s)" "$t" >>"${out}.backoff"
            fi
            sleep "$interval"
        done
    ) >/dev/null 2>&1 &
    echo $!
}
# The redirection above is load-bearing, not tidiness. Callers use
# `pid=$(thermal_watch ...)`, and a backgrounded subshell inherits the
# command substitution's stdout — so without it the substitution waits on
# a pipe the sampler holds open for the length of the run, and the caller
# hangs before the run it was about to guard ever starts.

# Called by the harness between segments. Echoes the load percentage the
# next segment should use, given the percentage the last one used.
# Consumes the backoff flag so each crossing costs exactly one step.
thermal_next_load_pct() {
    local out="$1" current="${2:-100}"
    if [[ -s "${out}.backoff" ]]; then
        rm -f "${out}.backoff"
        local next=$(( current - THERMAL_BACKOFF_STEP_PCT ))
        (( next < THERMAL_BACKOFF_FLOOR_PCT )) && next="$THERMAL_BACKOFF_FLOOR_PCT"
        printf '%d\n' "$next"
        return 0
    fi
    printf '%d\n' "$current"
}
# The redirection above is load-bearing, not tidiness. Callers use
# `pid=$(thermal_watch ...)`, and a backgrounded subshell inherits the
# command substitution's stdout — so without it the substitution waits on
# a pipe the sampler holds open for the length of the run, and the caller
# hangs before the run it was about to guard ever starts.

# Post-run verdict. Compares mean clock over the first fifth of the run
# against the last fifth: if the cores slowed down, the run spans two
# different machines and its numbers are not one measurement.
thermal_verdict() {
    local csv="$1"
    [[ -s "$csv" ]] || { echo "thermal: no samples"; return 0; }

    local backoffs=0
    [[ -s "${csv}.backoff" ]] && backoffs="$(wc -l <"${csv}.backoff")"

    # Backoffs are reported before anything else and regardless of sample
    # count. A short segment that backed off is exactly the case where the
    # caller most needs to know, and an early return for "too few samples
    # to judge clock drift" would have swallowed it silently.
    local rows
    rows="$(( $(wc -l <"$csv") - 1 ))"
    if (( rows < 10 )); then
        printf 'thermal: %d samples, too few to judge clock drift' "$rows"
        if (( backoffs > 0 )); then
            printf ' — but backed off %d time(s) at >=%d°C, so this segment is ' \
                "$backoffs" "$THERMAL_BACKOFF_C"
            printf 'load-limited\n'
        else
            printf '\n'
        fi
        return 0
    fi

    awk -F, -v back="$THERMAL_BACKOFF_C" -v warn="$THERMAL_WARN_C" \
            -v events="$backoffs" -v driftmin="$THERMAL_DRIFT_MIN_C" '
        NR == 1 { next }
        { n++; temp[n] = $2 + 0; mhz[n] = $3 + 0
          if (temp[n] > peak) peak = temp[n] }
        END {
            span = int(n / 5); if (span < 1) span = 1
            for (i = 1; i <= span; i++)          early += mhz[i]
            for (i = n - span + 1; i <= n; i++)  late  += mhz[i]
            early /= span; late /= span
            drop = early > 0 ? 100 * (early - late) / early : 0

            printf "thermal: peak %d°C, clock %d -> %d MHz (%.1f%% change)\n",
                   peak, early, late, -drop
            if (events > 0)
                printf "  LOAD-LIMITED: backed off %d time(s) at >=%d°C. This run " \
                       "did not hold one load for its duration, so it is not " \
                       "comparable to runs that did — re-run it or quote it as " \
                       "load-limited.\n", events, back
            else if (drop > 5 && peak >= driftmin)
                printf "  DEGRADED: cores slowed %.1f%% while the package sat at " \
                       "%d°C — that is thermal throttling, and the measurement " \
                       "spans two clock regimes\n", drop, peak
            else if (drop > 5)
                printf "  note: clock varied %.1f%% but peak was only %d°C (below " \
                       "%d°C) — core parking, not throttling\n", drop, peak, driftmin
            else if (peak >= warn)
                printf "  warm (peak %d°C >= %d°C) but clocks held\n", peak, warn
            else
                print "  ok: thermally stable, clocks held"
        }' "$csv"
}
