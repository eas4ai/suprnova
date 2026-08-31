#!/usr/bin/env python3
"""Validate the English manual and every localized mirror independently."""

from __future__ import annotations

import json
import posixpath
import re
import sys
from dataclasses import dataclass
from pathlib import Path, PurePosixPath
from urllib.parse import unquote, urlsplit, urlunsplit


LOCALES = ("de", "es", "fr", "ja", "pt-BR", "zh-Hans")
_HEADING = re.compile(r"^(?: {0,3}|\s*(?:[-+*]|\d+[.)])\s+)(#{1,6})(?:\s+|$)")
_LIST_ITEM = re.compile(r"^(\s*)([-+*]|\d+[.)])\s+")
_FENCE = re.compile(r"^ {0,3}(`{3,}|~{3,})(.*)$")
_REFERENCE_LINK = re.compile(r"^\s*\[[^]]+\]:\s*(?:<([^>]+)>|(\S+))")
_AUTOLINK = re.compile(r"<(https?://[^>]+|mailto:[^>]+)>", re.IGNORECASE)
_TABLE_DELIMITER_CELL = re.compile(r"^:?-+:?$")



@dataclass(frozen=True, order=True)
class Problem:
    """One mirror violation with enough identity for direct remediation."""

    locale: str
    file: str
    kind: str
    message: str

    def __str__(self) -> str:
        return f"{self.locale}/{self.file}: {self.kind}: {self.message}"


@dataclass(frozen=True)
class _MarkdownShape:
    headings: tuple[int, ...]
    fences: tuple[str, ...]
    tables: tuple[tuple[int, tuple[int, ...]], ...]
    lists: tuple[tuple[int, str], ...]
    links: tuple[str, ...]
    unclosed_fence: bool


def _read_utf8(path: Path, *, locale: str, file: str, problems: list[Problem]) -> str | None:
    try:
        return path.read_bytes().decode("utf-8")
    except OSError as error:
        problems.append(Problem(locale, file, "read", str(error)))
    except UnicodeDecodeError as error:
        problems.append(
            Problem(locale, file, "utf8", f"invalid UTF-8 at byte {error.start}")
        )
    return None


def _logical_path(path: PurePosixPath) -> PurePosixPath:
    parts = path.parts
    if len(parts) >= 3 and parts[0] == "manual" and parts[1] in LOCALES:
        if len(parts) == 3 and parts[2] == "changelog.md":
            return PurePosixPath("CHANGELOG.md")
        return PurePosixPath("manual", *parts[2:])
    return path


def _normalize_target(target: str, current_file: PurePosixPath) -> str:
    target = target.strip()
    if target.startswith("<") and target.endswith(">"):
        target = target[1:-1]
    split = urlsplit(target)
    if split.scheme or split.netloc:
        scheme = split.scheme.lower()
        netloc = split.netloc.lower()
        path = unquote(split.path)
        return urlunsplit((scheme, netloc, path, split.query, ""))

    raw_path = unquote(split.path).replace("\\", "/")
    if raw_path.startswith("/"):
        normalized_path = posixpath.normpath(raw_path)
    else:
        base = current_file.parent.as_posix()
        normalized_path = posixpath.normpath(posixpath.join(base, raw_path or current_file.name))
    logical = _logical_path(PurePosixPath(normalized_path)).as_posix()
    return logical + (f"?{split.query}" if split.query else "")


def _inline_link_targets(line: str) -> list[str]:
    targets: list[str] = []
    cursor = 0
    while True:
        marker = line.find("](", cursor)
        if marker < 0:
            break
        index = marker + 2
        while index < len(line) and line[index].isspace():
            index += 1
        if index >= len(line):
            break
        if line[index] == "<":
            end = line.find(">", index + 1)
            if end < 0:
                cursor = index + 1
                continue
            targets.append(line[index + 1 : end])
            cursor = end + 1
            continue
        end = index
        escaped = False
        while end < len(line):
            character = line[end]
            if escaped:
                escaped = False
            elif character == "\\":
                escaped = True
            elif character.isspace() or character == ")":
                break
            end += 1
        if end > index:
            targets.append(line[index:end])
        cursor = max(end + 1, index + 1)
    return targets


def _code_span_end(line: str, start: int) -> int | None:
    marker_length = 1
    while (
        start + marker_length < len(line)
        and line[start + marker_length] == "`"
    ):
        marker_length += 1

    index = start + marker_length
    while index < len(line):
        marker = line.find("`", index)
        if marker < 0:
            return None
        closing_length = 1
        while (
            marker + closing_length < len(line)
            and line[marker + closing_length] == "`"
        ):
            closing_length += 1
        if closing_length == marker_length:
            return marker + closing_length
        index = marker + closing_length
    return None


def _table_cells(line: str) -> tuple[str, ...] | None:
    stripped = line.strip()
    pipe_positions: list[int] = []
    index = 0
    while index < len(stripped):
        character = stripped[index]
        if character == "\\":
            index += 2
            continue
        if character == "`":
            code_end = _code_span_end(stripped, index)
            if code_end is not None:
                index = code_end
                continue
            while index < len(stripped) and stripped[index] == "`":
                index += 1
            continue
        if character == "|":
            pipe_positions.append(index)
        index += 1

    if not pipe_positions:
        return None
    cells: list[str] = []
    start = 0
    for pipe in pipe_positions:
        cells.append(stripped[start:pipe].strip())
        start = pipe + 1
    cells.append(stripped[start:].strip())
    if pipe_positions[0] == 0:
        cells.pop(0)
    if pipe_positions[-1] == len(stripped) - 1:
        cells.pop()
    return tuple(cells) if len(cells) >= 2 else None


def _is_table_delimiter(cells: tuple[str, ...] | None) -> bool:
    return cells is not None and all(
        _TABLE_DELIMITER_CELL.fullmatch(cell) is not None for cell in cells
    )


def _markdown_shape(text: str, current_file: PurePosixPath) -> _MarkdownShape:
    headings: list[int] = []
    fences: list[str] = []
    lists: list[tuple[int, str]] = []
    links: list[str] = []
    table_groups: list[list[int]] = []
    active_table: list[int] = []
    previous_table_cells: tuple[str, ...] | None = None
    closing_marker: str | None = None
    closing_length = 0

    for line in text.splitlines():
        fence = _FENCE.match(line)
        if fence is not None:
            marker = fence.group(1)
            if closing_marker is None:
                closing_marker = marker[0]
                closing_length = len(marker)
                info = fence.group(2).strip()
                fences.append(info.split(maxsplit=1)[0].lower() if info else "")
            elif marker[0] == closing_marker and len(marker) >= closing_length:
                closing_marker = None
                closing_length = 0
            if active_table:
                table_groups.append(active_table)
                active_table = []
            previous_table_cells = None
            continue
        if closing_marker is not None:
            previous_table_cells = None
            continue

        heading = _HEADING.match(line)
        if heading is not None:
            headings.append(len(heading.group(1)))
        item = _LIST_ITEM.match(line)
        if item is not None:
            marker = item.group(2)
            kind = "unordered" if marker in {"-", "+", "*"} else "ordered"
            lists.append((len(item.group(1).expandtabs(4)), kind))

        cells = _table_cells(line)
        if active_table:
            if cells is None:
                table_groups.append(active_table)
                active_table = []
            else:
                active_table.append(len(cells))
        elif (
            _is_table_delimiter(cells)
            and previous_table_cells is not None
            and len(previous_table_cells) == len(cells)
        ):
            active_table = [len(previous_table_cells), len(cells)]
        previous_table_cells = cells

        reference = _REFERENCE_LINK.match(line)
        if reference is not None:
            links.append(
                _normalize_target(
                    reference.group(1) or reference.group(2), current_file
                )
            )
        for target in _inline_link_targets(line):
            links.append(_normalize_target(target, current_file))
        for target in _AUTOLINK.findall(line):
            links.append(_normalize_target(target, current_file))

    if active_table:
        table_groups.append(active_table)
    tables = tuple((len(group), tuple(group)) for group in table_groups)
    return _MarkdownShape(
        headings=tuple(headings),
        fences=tuple(fences),
        tables=tables,
        lists=tuple(lists),
        links=tuple(links),
        unclosed_fence=closing_marker is not None,
    )


def _compare_shapes(
    english: _MarkdownShape,
    localized: _MarkdownShape,
    *,
    locale: str,
    file: str,
    problems: list[Problem],
) -> None:
    comparisons = (
        ("headings", english.headings, localized.headings),
        ("fences", english.fences, localized.fences),
        ("tables", english.tables, localized.tables),
        ("lists", english.lists, localized.lists),
        ("links", english.links, localized.links),
    )
    for kind, expected, actual in comparisons:
        if expected != actual:
            problems.append(
                Problem(locale, file, kind, f"expected {expected!r}, found {actual!r}")
            )
    if localized.unclosed_fence:
        problems.append(Problem(locale, file, "fences", "unclosed fenced code block"))


def _registered_utf8_files(root: Path, problems: list[Problem]) -> None:
    registry_path = root / "scripts/gate-steps.json"
    registry_text = _read_utf8(
        registry_path, locale="root", file="scripts/gate-steps.json", problems=problems
    )
    if registry_text is None:
        return
    try:
        payload = json.loads(registry_text)
    except json.JSONDecodeError as error:
        problems.append(
            Problem("root", "scripts/gate-steps.json", "registry", str(error))
        )
        return
    entries = payload.get("registered_files") if isinstance(payload, dict) else None
    if not isinstance(entries, list):
        problems.append(
            Problem(
                "root",
                "scripts/gate-steps.json",
                "registry",
                "registered_files must be a list",
            )
        )
        return
    for entry in entries:
        if not isinstance(entry, dict):
            continue
        relative = entry.get("path")
        checks = entry.get("checks")
        if (
            isinstance(relative, str)
            and isinstance(checks, list)
            and "utf8" in checks
        ):
            _read_utf8(root / relative, locale="root", file=relative, problems=problems)


def _validate_manual_structure(root: Path) -> tuple[list[Problem], int]:
    """Return mirror problems and the English inventory count from one run."""

    root = root.resolve()
    manual = root / "manual"
    problems: list[Problem] = []
    _registered_utf8_files(root, problems)

    try:
        english_names = sorted(
            path.name for path in manual.glob("*.md") if path.name != "README.md"
        )
    except OSError as error:
        return [Problem("en", "manual", "inventory", str(error))], 0
    sources: dict[str, tuple[Path, str]] = {
        name: (manual / name, name) for name in english_names
    }
    sources["CHANGELOG.md"] = (
        root / "CHANGELOG.md",
        "changelog.md",
    )
    expected_locale_names = {
        locale_name for _path, locale_name in sources.values()
    }

    english_shapes: dict[str, _MarkdownShape] = {}
    for source_name, (source_path, _locale_name) in sources.items():
        text = _read_utf8(
            source_path, locale="en", file=source_name, problems=problems
        )
        if text is None:
            continue
        current_file = PurePosixPath(source_path.relative_to(root).as_posix())
        shape = _markdown_shape(text, current_file)
        english_shapes[source_name] = shape
        if shape.unclosed_fence:
            problems.append(
                Problem("en", source_name, "fences", "unclosed fenced code block")
            )

    for locale in LOCALES:
        locale_root = manual / locale
        actual = {path.name for path in locale_root.glob("*.md")}
        for missing in sorted(expected_locale_names - actual):
            problems.append(
                Problem(locale, missing, "inventory-missing", "localized chapter is missing")
            )
        for extra in sorted(actual - expected_locale_names):
            problems.append(
                Problem(locale, extra, "inventory-extra", "no English source chapter")
            )
        for source_name, (_source_path, locale_name) in sources.items():
            if locale_name not in actual or source_name not in english_shapes:
                continue
            localized_path = locale_root / locale_name
            localized_text = _read_utf8(
                localized_path,
                locale=locale,
                file=locale_name,
                problems=problems,
            )
            if localized_text is None:
                continue
            current_file = PurePosixPath(localized_path.relative_to(root).as_posix())
            localized_shape = _markdown_shape(localized_text, current_file)
            _compare_shapes(
                english_shapes[source_name],
                localized_shape,
                locale=locale,
                file=locale_name,
                problems=problems,
            )

    return sorted(problems), len(sources)


def check_manual_structure(root: Path) -> list[Problem]:
    """Return all independently observed English/locale mirror problems."""

    problems, _source_count = _validate_manual_structure(root)
    return problems


def main() -> int:
    problems, source_count = _validate_manual_structure(Path.cwd())
    if problems:
        for problem in problems:
            print(problem)
        print(f"manual mirror structure failed: {len(problems)} problem(s)", file=sys.stderr)
        return 1
    print(
        f"manual mirror structure current: {source_count} sources x "
        f"{len(LOCALES)} locales"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
