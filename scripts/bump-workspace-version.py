#!/usr/bin/env python3
"""Atomically bump workspace version metadata, internal path requirements,
and the version references README.md carries in prose."""

from __future__ import annotations

import argparse
import json
import os
import re
import stat
import subprocess
import sys
import tempfile
import tomllib
from dataclasses import dataclass
from pathlib import Path


SEMVER = re.compile(
    r"^(0|[1-9][0-9]*)\."
    r"(0|[1-9][0-9]*)\."
    r"(0|[1-9][0-9]*)"
    r"(?:-(?P<prerelease>[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?"
    r"(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$"
)
DEPENDENCY_TABLES = ("dependencies", "dev-dependencies", "build-dependencies")


@dataclass(frozen=True)
class PathRequirement:
    manifest: Path
    key: str
    dependency_path: Path
    version: str


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("version", help="new semantic version")
    parser.add_argument(
        "--root",
        type=Path,
        default=Path.cwd(),
        help="workspace root (default: current directory)",
    )
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument(
        "--verify",
        action="store_true",
        help="verify the workspace is already consistently versioned",
    )
    mode.add_argument(
        "--validate-only",
        action="store_true",
        help="validate the version argument without reading or editing manifests",
    )
    return parser.parse_args()


def validate_version(version: str) -> None:
    match = SEMVER.fullmatch(version)
    if match is None:
        raise ValueError(f"'{version}' is not a valid semantic version")
    prerelease = match.group("prerelease")
    if prerelease is not None and any(
        len(identifier) > 1
        and identifier.startswith("0")
        and identifier.isdigit()
        for identifier in prerelease.split(".")
    ):
        raise ValueError(
            f"'{version}' is not a valid semantic version: numeric "
            "prerelease identifiers must not contain leading zeroes"
        )


def load_toml(path: Path) -> dict[str, object]:
    with path.open("rb") as handle:
        return tomllib.load(handle)


def cargo_metadata(root: Path) -> dict[str, object]:
    result = subprocess.run(
        [
            "cargo",
            "metadata",
            "--no-deps",
            "--format-version",
            "1",
            "--manifest-path",
            str(root / "Cargo.toml"),
        ],
        cwd=root,
        check=True,
        capture_output=True,
        text=True,
    )
    return json.loads(result.stdout)


def workspace_manifests(root: Path) -> list[Path]:
    metadata = cargo_metadata(root)
    return sorted(
        Path(package["manifest_path"]).resolve()
        for package in metadata["packages"]
    )


def dependency_tables(document: dict[str, object]) -> list[dict[str, object]]:
    tables: list[dict[str, object]] = []
    for name in DEPENDENCY_TABLES:
        table = document.get(name)
        if isinstance(table, dict):
            tables.append(table)

    targets = document.get("target")
    if isinstance(targets, dict):
        for target in targets.values():
            if not isinstance(target, dict):
                continue
            for name in DEPENDENCY_TABLES:
                table = target.get(name)
                if isinstance(table, dict):
                    tables.append(table)
    return tables


def internal_path_requirements(root: Path) -> list[PathRequirement]:
    manifests = workspace_manifests(root)
    workspace_dirs = {manifest.parent.resolve() for manifest in manifests}
    requirements: list[PathRequirement] = []

    for manifest in manifests:
        document = load_toml(manifest)
        for table in dependency_tables(document):
            for key, dependency in table.items():
                if not isinstance(dependency, dict):
                    continue
                path = dependency.get("path")
                version = dependency.get("version")
                if not isinstance(path, str) or not isinstance(version, str):
                    continue
                dependency_path = (manifest.parent / path).resolve()
                if dependency_path not in workspace_dirs:
                    continue
                requirements.append(
                    PathRequirement(manifest, key, dependency_path, version)
                )

    return requirements


def replace_workspace_version(source: str, version: str) -> str:
    lines = source.splitlines(keepends=True)
    in_workspace_package = False
    replacements = 0

    for index, line in enumerate(lines):
        section = re.match(r"^\s*\[([^]]+)]\s*(?:#.*)?$", line)
        if section:
            in_workspace_package = section.group(1) == "workspace.package"
            continue
        if not in_workspace_package:
            continue
        updated, count = re.subn(
            r'^(\s*version\s*=\s*")[^"]+(")',
            rf"\g<1>{version}\g<2>",
            line,
            count=1,
        )
        if count:
            lines[index] = updated
            replacements += count

    if replacements != 1:
        raise ValueError(
            "expected exactly one workspace.package.version in Cargo.toml"
        )
    return "".join(lines)


#: Embeddable semver fragment, for building larger patterns. Distinct from the
#: anchored, compiled ``SEMVER`` above, which validates a version on its own.
SEMVER_FRAGMENT = r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(?:-[0-9A-Za-z.-]+)?"

#: The dependency-tag pin, ``tag = "vX.Y.Z"``. Every README that tells a
#: consumer how to depend on this repo carries one, because the git tag *is*
#: the release.
RULE_DEP_TAG = "dep_tag"
#: The ``cargo install --tag vX.Y.Z`` line. Root README only.
RULE_INSTALL_TAG = "install_tag"
#: The "Suprnova X.Y requires Rust …" MSRV sentence. Root README only.
RULE_MSRV_MINOR = "msrv_minor"

#: Every README whose prose pins a version, and which rules must match in it.
#:
#: The root README was already covered; the three adapter READMEs were not,
#: which is how they advertised v0.6.0 while v0.7.2 shipped — the identical
#: failure this function was written to stop, reintroduced by a file the list
#: did not name. `assert_all_versioned_readmes_listed` now fails the release
#: if a README pins a tag without appearing here, so a new adapter crate
#: cannot repeat it a third time.
VERSIONED_READMES: dict[str, tuple[str, ...]] = {
    "README.md": (RULE_INSTALL_TAG, RULE_DEP_TAG, RULE_MSRV_MINOR),
    "framework/README.md": (RULE_INSTALL_TAG, RULE_DEP_TAG),
    "crates/suprnova-payments-stripe/README.md": (RULE_DEP_TAG,),
    "crates/suprnova-payments-paddle/README.md": (RULE_DEP_TAG,),
    "crates/suprnova-web-push/README.md": (RULE_DEP_TAG,),
}


def replace_readme_versions(
    source: str, version: str, rules: tuple[str, ...], label: str = "README.md"
) -> str:
    """Rewrite the version references a README carries in prose.

    Manifests are bumped atomically on every release; these were not, which is
    how a README advertised v0.6.0 while v0.7.0 shipped. Each rule named for
    the file must match at least once, so a reworded README fails the release
    loudly instead of silently going stale again.
    """
    major_minor = ".".join(version.split(".")[:2])
    rewrites = {
        RULE_INSTALL_TAG: (rf"(--tag v){SEMVER_FRAGMENT}", rf"\g<1>{version}"),
        RULE_DEP_TAG: (rf'(tag = "v){SEMVER_FRAGMENT}(")', rf"\g<1>{version}\g<5>"),
        RULE_MSRV_MINOR: (
            r"(Suprnova )(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)( requires Rust)",
            rf"\g<1>{major_minor}\g<4>",
        ),
    }
    for rule in rules:
        pattern, replacement = rewrites[rule]
        source, count = re.subn(pattern, replacement, source)
        if count == 0:
            raise ValueError(
                f"{label} matches nothing for {rule} ({pattern!r}); update "
                "VERSIONED_READMES alongside the README wording"
            )
    return source


def assert_all_versioned_readmes_listed(root: Path) -> None:
    """Fail if any tracked README pins a tag without being in the list.

    The list is only protective for files it names. A new adapter crate ships
    with a `tag = "vX.Y.Z"` install snippet and would silently freeze at
    whatever version it was written against — so discover them instead of
    trusting the list to stay complete.
    """
    pinned = re.compile(rf'tag = "v{SEMVER_FRAGMENT}"')
    unlisted = []
    for readme in sorted(root.rglob("README.md")):
        relative = readme.relative_to(root).as_posix()
        if relative in VERSIONED_READMES:
            continue
        # Scaffolder templates and vendored reference trees are not ours to
        # bump: templates interpolate the tag at scaffold time, and
        # `reference/` is gitignored third-party source.
        if any(
            part in {"target", "node_modules", "reference", "templates", ".git"}
            for part in readme.relative_to(root).parts
        ):
            continue
        if pinned.search(readme.read_text(encoding="utf-8")):
            unlisted.append(relative)
    if unlisted:
        raise ValueError(
            "these READMEs pin a release tag but are not in VERSIONED_READMES, "
            "so they will go stale at the next release: " + ", ".join(unlisted)
        )


def inline_dependency(line: str, key: str) -> dict[str, object] | None:
    if not re.match(rf'^\s*{re.escape(key)}\s*=\s*\{{', line):
        return None
    try:
        parsed = tomllib.loads(f"[dependencies]\n{line}")
    except tomllib.TOMLDecodeError:
        return None
    dependency = parsed.get("dependencies", {}).get(key)
    return dependency if isinstance(dependency, dict) else None


def replace_path_requirement(
    source: str,
    requirement: PathRequirement,
    version: str,
) -> str:
    lines = source.splitlines(keepends=True)
    replacements = 0

    for index, line in enumerate(lines):
        dependency = inline_dependency(line, requirement.key)
        if dependency is None:
            continue
        path = dependency.get("path")
        current_version = dependency.get("version")
        if not isinstance(path, str) or current_version != requirement.version:
            continue
        resolved = (requirement.manifest.parent / path).resolve()
        if resolved != requirement.dependency_path:
            continue
        updated, count = re.subn(
            r'(\bversion\s*=\s*")[^"]+(")',
            rf"\g<1>{version}\g<2>",
            line,
            count=1,
        )
        if count:
            lines[index] = updated
            replacements += count
            break

    if replacements != 1:
        relative = requirement.manifest.name
        raise ValueError(
            f"could not safely rewrite {requirement.key} in {relative}; "
            "internal versioned path dependencies must use an inline table"
        )
    return "".join(lines)


def verify(root: Path, version: str) -> list[PathRequirement]:
    root_document = load_toml(root / "Cargo.toml")
    workspace = root_document.get("workspace")
    package = workspace.get("package") if isinstance(workspace, dict) else None
    actual = package.get("version") if isinstance(package, dict) else None
    if actual != version:
        raise ValueError(
            f"workspace.package.version is {actual!r}, expected {version!r}"
        )

    metadata = cargo_metadata(root)
    mismatched_packages = [
        f"{item['name']}={item['version']}"
        for item in metadata["packages"]
        if item["version"] != version
    ]
    if mismatched_packages:
        raise ValueError(
            "workspace packages did not inherit the release version: "
            + ", ".join(mismatched_packages)
        )

    requirements = internal_path_requirements(root)
    mismatched_requirements = [
        f"{requirement.manifest.relative_to(root)}:{requirement.key}="
        f"{requirement.version}"
        for requirement in requirements
        if requirement.version != version
    ]
    if mismatched_requirements:
        raise ValueError(
            "internal path requirements do not match the workspace version: "
            + ", ".join(mismatched_requirements)
        )

    # The rewrite is idempotent at the target version, so "applying it
    # changes nothing" is exactly "this README already carries this version".
    assert_all_versioned_readmes_listed(root)
    for relative, rules in VERSIONED_READMES.items():
        readme_source = (root / relative).read_text(encoding="utf-8")
        if replace_readme_versions(readme_source, version, rules, relative) != readme_source:
            raise ValueError(
                f"{relative} version references do not match the workspace version"
            )
    return requirements


def write_all_atomically(updated: dict[Path, str]) -> None:
    staged: list[tuple[Path, Path]] = []
    try:
        for path, source in updated.items():
            descriptor, temp_name = tempfile.mkstemp(
                prefix=f".{path.name}.", suffix=".tmp", dir=path.parent
            )
            temp_path = Path(temp_name)
            with os.fdopen(descriptor, "w", encoding="utf-8") as handle:
                handle.write(source)
            os.chmod(temp_path, stat.S_IMODE(path.stat().st_mode))
            staged.append((path, temp_path))
        for path, temp_path in staged:
            os.replace(temp_path, path)
    finally:
        for _, temp_path in staged:
            temp_path.unlink(missing_ok=True)


def bump(root: Path, version: str) -> list[Path]:
    requirements = internal_path_requirements(root)
    if not requirements:
        raise ValueError("workspace has no versioned internal path dependencies")

    assert_all_versioned_readmes_listed(root)
    readmes = {relative: root / relative for relative in VERSIONED_READMES}
    paths = {
        root / "Cargo.toml",
        *readmes.values(),
        *(item.manifest for item in requirements),
    }
    originals = {path: path.read_text(encoding="utf-8") for path in paths}
    updated = dict(originals)
    updated[root / "Cargo.toml"] = replace_workspace_version(
        updated[root / "Cargo.toml"], version
    )
    for relative, readme in readmes.items():
        updated[readme] = replace_readme_versions(
            updated[readme], version, VERSIONED_READMES[relative], relative
        )
    for requirement in requirements:
        updated[requirement.manifest] = replace_path_requirement(
            updated[requirement.manifest], requirement, version
        )

    try:
        write_all_atomically(updated)
        verify(root, version)
    except Exception:
        write_all_atomically(originals)
        raise
    return sorted(path for path in paths if updated[path] != originals[path])


def main() -> int:
    args = parse_args()
    try:
        validate_version(args.version)
        if args.validate_only:
            return 0

        root = args.root.resolve()
        if args.verify:
            requirements = verify(root, args.version)
            print(f"workspace.package.version={args.version}")
            for requirement in requirements:
                print(
                    f"{requirement.manifest.relative_to(root)}:"
                    f"{requirement.key}={requirement.version}"
                )
            print(f"internal path requirements={len(requirements)}")
            return 0

        for path in bump(root, args.version):
            print(path.relative_to(root))
        return 0
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
