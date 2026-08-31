#!/usr/bin/env bash
# Release-gate check: translated manuals must not lag the English manual.
#
# manual/{de,es,fr,ja,pt-BR,zh-Hans}/ is a full 1:1 mirror of the top-level
# English chapters. The lock records the English blob and every translated blob
# used for one translation run. Checking only the English blob is insufficient:
# a lock-only restamp can otherwise bless unchanged, stale locale content.
#
# scripts/gate.sh runs this only under --full. Day-to-day pushes are exempt so
# translations may lag during an implementation wave.
#
# Updating the lock after real translation output:
#   scripts/check-manual-translations.sh --stamp
#   scripts/check-manual-translations.sh --stamp validation.md mail.md
#
# For a deliberately translation-neutral English edit, name every affected
# chapter and make the waiver explicit:
#   scripts/check-manual-translations.sh --stamp --translation-neutral mail.md
# The waiver is rejected for a full-tree stamp because that would hide scope.

set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

LOCK=".manual-translations.lock"
LOCALES=(de es fr ja pt-BR zh-Hans)

chapters() {
    for file in manual/*.md; do
        name="$(basename "$file")"
        [[ "$name" == "README.md" ]] || echo "$name"
    done
}

source_path() {
    local name="$1"
    if [[ "$name" == "CHANGELOG.md" ]]; then
        echo "CHANGELOG.md"
    else
        echo "manual/$name"
    fi
}

locale_path() {
    local locale="$1" name="$2"
    if [[ "$name" == "CHANGELOG.md" ]]; then
        echo "manual/$locale/changelog.md"
    else
        echo "manual/$locale/$name"
    fi
}

hash_file() {
    git hash-object "$1"
}

declare -A old_english=()
declare -A old_de=()
declare -A old_es=()
declare -A old_fr=()
declare -A old_ja=()
declare -A old_pt=()
declare -A old_zh=()

if [[ -f "$LOCK" ]]; then
    while read -r name english de_hash es_hash fr_hash ja_hash pt_hash zh_hash _extra; do
        [[ -n "$name" && "${name:0:1}" != "#" ]] || continue
        old_english["$name"]="${english:-}"
        old_de["$name"]="${de_hash:-}"
        old_es["$name"]="${es_hash:-}"
        old_fr["$name"]="${fr_hash:-}"
        old_ja["$name"]="${ja_hash:-}"
        old_pt["$name"]="${pt_hash:-}"
        old_zh["$name"]="${zh_hash:-}"
    done < "$LOCK"
fi

if [[ "${1:-}" == "--stamp" ]]; then
    shift
    translation_neutral=0
    if [[ "${1:-}" == "--translation-neutral" ]]; then
        translation_neutral=1
        shift
    fi
    if (( translation_neutral == 1 && $# == 0 )); then
        echo "--translation-neutral requires explicit chapter names" >&2
        exit 1
    fi

    if (( $# > 0 )); then
        targets=("$@")
    else
        mapfile -t targets < <(chapters)
        targets+=(CHANGELOG.md)
    fi

    declare -A selected=()
    for name in "${targets[@]}"; do
        selected["$name"]=1
        source="$(source_path "$name")"
        [[ -f "$source" ]] || { echo "no such translation source: $source" >&2; exit 1; }
        for locale in "${LOCALES[@]}"; do
            translated="$(locale_path "$locale" "$name")"
            [[ -f "$translated" ]] || { echo "missing translation: $translated" >&2; exit 1; }
        done

        new_english="$(hash_file "$source")"
        current_locale_hashes=(
            "$(hash_file "$(locale_path de "$name")")"
            "$(hash_file "$(locale_path es "$name")")"
            "$(hash_file "$(locale_path fr "$name")")"
            "$(hash_file "$(locale_path ja "$name")")"
            "$(hash_file "$(locale_path pt-BR "$name")")"
            "$(hash_file "$(locale_path zh-Hans "$name")")"
        )
        previous_locale_hashes=(
            "${old_de[$name]:-}" "${old_es[$name]:-}" "${old_fr[$name]:-}"
            "${old_ja[$name]:-}" "${old_pt[$name]:-}" "${old_zh[$name]:-}"
        )

        if [[ -n "${old_english[$name]:-}" && "${old_english[$name]}" != "$new_english" ]]; then
            locale_changed=0
            prior_complete=1
            for index in "${!current_locale_hashes[@]}"; do
                [[ -n "${previous_locale_hashes[$index]}" ]] || prior_complete=0
                [[ "${current_locale_hashes[$index]}" != "${previous_locale_hashes[$index]}" ]] && locale_changed=1
            done
            if (( prior_complete == 1 && locale_changed == 0 && translation_neutral == 0 )); then
                echo "refusing lock-only restamp for $name: English changed but no locale blob changed" >&2
                echo "translate it, or use --stamp --translation-neutral $name for a content-neutral edit" >&2
                exit 1
            fi
        fi

        old_english["$name"]="$new_english"
        old_de["$name"]="${current_locale_hashes[0]}"
        old_es["$name"]="${current_locale_hashes[1]}"
        old_fr["$name"]="${current_locale_hashes[2]}"
        old_ja["$name"]="${current_locale_hashes[3]}"
        old_pt["$name"]="${current_locale_hashes[4]}"
        old_zh["$name"]="${current_locale_hashes[5]}"
    done

    {
        echo "# English and locale blob hashes from the same manual translation run."
        echo "# Columns: source English de es fr ja pt-BR zh-Hans"
        while read -r name; do
            [[ -n "${old_english[$name]:-}" ]] || continue
            if [[ "$name" == "CHANGELOG.md" || -f "manual/$name" ]]; then
                printf '%s %s %s %s %s %s %s %s\n' \
                    "$name" "${old_english[$name]}" "${old_de[$name]:-}" \
                    "${old_es[$name]:-}" "${old_fr[$name]:-}" "${old_ja[$name]:-}" \
                    "${old_pt[$name]:-}" "${old_zh[$name]:-}"
            fi
        done < <(printf '%s\n' "${!old_english[@]}" | sort)
    } > "$LOCK"

    echo "stamped $LOCK ($(grep -c '^[^#]' "$LOCK") sources, English + ${#LOCALES[@]} locales)"
    exit 0
fi

[[ -f "$LOCK" ]] || {
    echo "$LOCK missing - translate the manual, then run the stamp command" >&2
    exit 1
}

problems=0
declare -A expected=()
while read -r name english de_hash es_hash fr_hash ja_hash pt_hash zh_hash _extra; do
    [[ -n "$name" && "${name:0:1}" != "#" ]] || continue
    expected["$name"]=1
    source="$(source_path "$name")"
    if [[ ! -f "$source" ]]; then
        echo "ORPHAN   $name in $LOCK (source is gone)"
        problems=$((problems + 1))
        continue
    fi
    current_english="$(hash_file "$source")"
    if [[ "$english" != "$current_english" ]]; then
        echo "STALE    $name (English changed since translation)"
        problems=$((problems + 1))
    fi

    recorded_hashes=("${de_hash:-}" "${es_hash:-}" "${fr_hash:-}" "${ja_hash:-}" "${pt_hash:-}" "${zh_hash:-}")
    for index in "${!LOCALES[@]}"; do
        locale="${LOCALES[$index]}"
        translated="$(locale_path "$locale" "$name")"
        if [[ ! -f "$translated" ]]; then
            echo "MISSING  $translated"
            problems=$((problems + 1))
        elif [[ -z "${recorded_hashes[$index]}" ]]; then
            echo "UNLOCKED $translated (lock predates locale-hash verification)"
            problems=$((problems + 1))
        elif [[ "${recorded_hashes[$index]}" != "$(hash_file "$translated")" ]]; then
            echo "STALE    $translated (locale changed after the stamped translation run)"
            problems=$((problems + 1))
        fi
    done
done < "$LOCK"

while read -r name; do
    if [[ -z "${expected[$name]:-}" ]]; then
        echo "UNLOCKED $name (no lock entry)"
        problems=$((problems + 1))
    fi
done < <(chapters; echo CHANGELOG.md)

for locale in "${LOCALES[@]}"; do
    for translated in manual/"$locale"/*.md; do
        [[ -f "$translated" ]] || continue
        name="$(basename "$translated")"
        [[ "$name" == "changelog.md" ]] && continue
        [[ -f "manual/$name" ]] || {
            echo "ORPHAN   $translated (no English chapter)"
            problems=$((problems + 1))
        }
    done
done

if (( problems > 0 )); then
    echo >&2
    echo "$problems problem(s): translated manuals do not match the stamped source set" >&2
    echo "translate the files, then stamp and commit English plus every locale hash" >&2
    exit 1
fi

echo "manual translations current: $(grep -c '^[^#]' "$LOCK") sources x ${#LOCALES[@]} locales"
