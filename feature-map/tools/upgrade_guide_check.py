#!/usr/bin/env python3
"""Check a release's upgrade chapter against its changelog section (MAN-108).

Usage: upgrade_guide_check.py <version>|--newest [<repo root>]

Reads the `## <version> - <date>` section of CHANGELOG.md (`--newest`: the
first such section, the release being prepared) and the chapter
manual/upgrade-<major>.<minor>.md. Every entry under `### Changed` and
`### Removed` in that section, identified by its bold lead sentence, must
head a section of the chapter (`### <lead>` or deeper); every entry under
`### Fixed` must either head a section (it changes a behaviour the manual
documented) or appear as a bold list item of the chapter (`- **<lead>**`,
under the chapter's list of fixes with no upgrade step), so that the
judgment on each fix is written down where the check can see it; and the
chapter must be linked from manual/documentation.md by a Markdown link.
The check reads Markdown structure: a heading, a list item or a link
inside a fenced code block or an HTML comment is example text and counts
for nothing. Prints each missing entry and exits 1 when one is missing;
exits 0 when the chapter accounts for them all. Written for a person and
an agent alike: the bold lead of a changelog entry is the heading or the
list item to search for.
"""
import re
import sys
from pathlib import Path


def newest_version(changelog: str) -> str:
    """The version of the changelog's first release section."""
    match = re.search(r"^## (\d+\.\d+\.\d+) - \S+\s*$", changelog, re.M)
    if not match:
        raise SystemExit("CHANGELOG.md has no release section")
    return match.group(1)


def release_section(changelog: str, version: str) -> str:
    """The lines of the changelog's section for `version`."""
    start = re.search(rf"^## {re.escape(version)} - \S+\s*$", changelog, re.M)
    if not start:
        raise SystemExit(f"CHANGELOG.md has no section for {version}")
    rest = changelog[start.end():]
    end = re.search(r"^## \d", rest, re.M)
    return rest[: end.start()] if end else rest


def outside_comments(line: str, in_comment: bool) -> tuple[str, bool]:
    """`line` without its HTML comments, and whether a comment stays open
    after it. A `<!--` inside a code span is code, not a comment."""
    masked = re.sub(r"`[^`]*`", lambda span: "x" * len(span.group()), line)
    kept: list[str] = []
    position = 0
    while position < len(line):
        if in_comment:
            end = masked.find("-->", position)
            if end < 0:
                return "".join(kept), True
            position = end + 3
            in_comment = False
        else:
            start = masked.find("<!--", position)
            if start < 0:
                kept.append(line[position:])
                break
            kept.append(line[position:start])
            position = start + 4
            in_comment = True
    return "".join(kept), in_comment


def prose_lines(text: str) -> list[str]:
    """The lines of a Markdown document outside fenced code blocks and HTML
    comments. A fence line is dropped with its block, and a comment is cut
    out of its line, so a heading, a list item or a link that is example
    text never passes for the chapter's structure."""
    lines: list[str] = []
    fence: str | None = None
    in_comment = False
    for line in text.splitlines():
        stripped = line.lstrip(" ")
        indent = len(line) - len(stripped)
        if fence is not None:
            closing = stripped.rstrip()
            if indent < 4 and closing.startswith(fence) and closing.strip(fence[0]) == "":
                fence = None
            continue
        if not in_comment:
            opening = re.match(r"(`{3,}|~{3,})", stripped)
            if opening and indent < 4:
                fence = opening.group(1)
                continue
        kept, in_comment = outside_comments(line, in_comment)
        lines.append(kept)
    return lines


def bold_item(lines: list[str], index: int) -> str | None:
    """The bold text opening the list item at `index`, or None.

    A bold text that wraps onto the item's continuation lines is joined
    with single spaces, so the text to search for is the sentence as read,
    not as the file breaks it.
    """
    entry = lines[index]
    if not entry.startswith("- **"):
        return None
    following = index + 1
    while "**" not in entry[4:] and following < len(lines) and lines[following].startswith("  "):
        entry = f"{entry} {lines[following].strip()}"
        following += 1
    match = re.match(r"- \*\*(.+?)\*\*", entry)
    return match.group(1).strip() if match else None


def bold_leads(section: str, headings: tuple[str, ...]) -> list[str]:
    """The bold lead sentence of every entry under the given `###` headings."""
    leads: list[str] = []
    current = None
    lines = prose_lines(section)
    for index, line in enumerate(lines):
        if line.startswith("### "):
            current = line[4:].strip()
            continue
        if current in headings:
            lead = bold_item(lines, index)
            if lead is not None:
                leads.append(lead)
    return leads


def chapter_items(chapter: str) -> set[str]:
    """The bold text of every list item of the chapter, outside code and comments."""
    lines = prose_lines(chapter)
    return {item for item in (bold_item(lines, index) for index in range(len(lines))) if item is not None}


def chapter_headings(chapter: str) -> set[str]:
    """Every heading of the chapter, without its marks, outside code and comments."""
    return {
        re.sub(r"^#+\s+", "", line).strip()
        for line in prose_lines(chapter)
        if re.match(r"^#{1,6}\s+\S", line)
    }


def links_chapter(index: str, chapter_name: str) -> bool:
    """Whether the index holds a Markdown link to the chapter, outside code
    and comments; a fragment after the file name is allowed."""
    pattern = re.compile(r"\]\((?:\./)?" + re.escape(chapter_name) + r"(?:#[^)\s]*)?\)")
    return any(pattern.search(line) for line in prose_lines(index))


def main() -> int:
    if len(sys.argv) < 2:
        print(__doc__)
        return 2
    version = sys.argv[1]
    root = Path(sys.argv[2]) if len(sys.argv) > 2 else Path(__file__).resolve().parents[2]
    changelog = (root / "CHANGELOG.md").read_text()
    if version == "--newest":
        version = newest_version(changelog)
    major_minor = ".".join(version.split(".")[:2])
    chapter_path = root / "manual" / f"upgrade-{major_minor}.md"
    section = release_section(changelog, version)
    leads = bold_leads(section, ("Changed", "Removed"))
    fixed = bold_leads(section, ("Fixed",))
    problems: list[str] = []
    if not chapter_path.exists():
        problems.append(f"{chapter_path.relative_to(root)} does not exist")
        headings: set[str] = set()
        items: set[str] = set()
    else:
        chapter = chapter_path.read_text()
        headings = chapter_headings(chapter)
        items = chapter_items(chapter)
    index = (root / "manual" / "documentation.md").read_text()
    if not links_chapter(index, chapter_path.name):
        problems.append(f"manual/documentation.md does not link {chapter_path.name}")
    missing = [lead for lead in leads if lead not in headings]
    for lead in missing:
        problems.append(f"no section for the changelog entry: {lead}")
    unaccounted = [lead for lead in fixed if lead not in headings and lead not in items]
    for lead in unaccounted:
        problems.append(f"no section and no listing for the fixed entry: {lead}")
    covered = len(leads) - len(missing)
    accounted = len(fixed) - len(unaccounted)
    if problems:
        print(
            f"upgrade guide for {version}: {len(problems)} problem(s), {covered} of {len(leads)} entries covered, "
            f"{accounted} of {len(fixed)} Fixed entries accounted for"
        )
        for problem in problems:
            print(f"  {problem}")
        return 1
    print(
        f"upgrade guide for {version}: all {len(leads)} Changed and Removed entries covered, "
        f"{len(fixed)} Fixed entries accounted for, chapter linked"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
