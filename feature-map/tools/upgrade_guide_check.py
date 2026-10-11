#!/usr/bin/env python3
"""Check a release's upgrade chapter against its changelog section (MAN-108).

Usage: upgrade_guide_check.py <version> [<repo root>]

Reads the `## <version> - <date>` section of CHANGELOG.md and the chapter
manual/upgrade-<major>.<minor>.md. Every entry under `### Changed` and
`### Removed` in that section, identified by its bold lead sentence, must
head a section of the chapter (`### <lead>` or deeper), and the chapter
must be linked from manual/documentation.md. Prints each missing entry and
exits 1 when one is missing; exits 0 when the chapter covers them all.
Written for a person and an agent alike: the bold lead of a changelog
entry is the heading to search for in the chapter.
"""
import re
import sys
from pathlib import Path


def release_section(changelog: str, version: str) -> str:
    """The lines of the changelog's section for `version`."""
    start = re.search(rf"^## {re.escape(version)} - \S+\s*$", changelog, re.M)
    if not start:
        raise SystemExit(f"CHANGELOG.md has no section for {version}")
    rest = changelog[start.end():]
    end = re.search(r"^## \d", rest, re.M)
    return rest[: end.start()] if end else rest


def bold_leads(section: str, headings: tuple[str, ...]) -> list[str]:
    """The bold lead sentence of every entry under the given `###` headings."""
    leads: list[str] = []
    current = None
    for line in section.splitlines():
        if line.startswith("### "):
            current = line[4:].strip()
            continue
        if current in headings and line.startswith("- **"):
            match = re.match(r"- \*\*(.+?)\*\*", line)
            if match:
                leads.append(match.group(1).strip())
    return leads


def chapter_headings(chapter: str) -> set[str]:
    """Every heading of the chapter, without its marks."""
    return {
        re.sub(r"^#+\s*", "", line).strip()
        for line in chapter.splitlines()
        if line.startswith("#")
    }


def main() -> int:
    if len(sys.argv) < 2:
        print(__doc__)
        return 2
    version = sys.argv[1]
    root = Path(sys.argv[2]) if len(sys.argv) > 2 else Path(__file__).resolve().parents[2]
    major_minor = ".".join(version.split(".")[:2])
    chapter_path = root / "manual" / f"upgrade-{major_minor}.md"
    changelog = (root / "CHANGELOG.md").read_text()
    section = release_section(changelog, version)
    leads = bold_leads(section, ("Changed", "Removed"))
    problems: list[str] = []
    if not chapter_path.exists():
        problems.append(f"{chapter_path.relative_to(root)} does not exist")
        headings: set[str] = set()
    else:
        headings = chapter_headings(chapter_path.read_text())
    index = (root / "manual" / "documentation.md").read_text()
    if f"({chapter_path.name})" not in index:
        problems.append(f"manual/documentation.md does not link {chapter_path.name}")
    missing = [lead for lead in leads if lead not in headings]
    for lead in missing:
        problems.append(f"no section for the changelog entry: {lead}")
    if problems:
        print(f"upgrade guide for {version}: {len(problems)} problem(s), {len(leads) - len(missing)} of {len(leads)} entries covered")
        for problem in problems:
            print(f"  {problem}")
        return 1
    print(f"upgrade guide for {version}: all {len(leads)} Changed and Removed entries covered, chapter linked")
    return 0


if __name__ == "__main__":
    sys.exit(main())
