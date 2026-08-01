#!/usr/bin/env python3
"""Verify the isolated consumer resolves the pinned storage security graph."""

from __future__ import annotations

import json
import sys
from pathlib import Path


OPENDAL_REV = "88717391eb72c9839d3f8e79fccad9f22fc3a1b4"
REQSIGN_REV = "b49cd2996b9d2d9944e84481f8835ff55b188b97"

# The consumer this script resolves takes `filesystem` and nothing else,
# which is S3 only.
REQSIGN_REQUIRED = {
    "reqsign-aws-v4",
    "reqsign-core",
    "reqsign-file-read-tokio",
}

# Azure and GCS moved behind `filesystem-azure` / `filesystem-gcs` in
# 0.9.0, and these two crates are why: they are the only ones that enable
# `reqsign-core/jwt`, the feature `reqsign-core`'s optional `rsa` sits
# behind — RUSTSEC-2023-0071, the Marvin timing attack, unfixed upstream.
# Either one reappearing in a default-feature graph means the gating
# regressed and `rsa` is back. `check-feature-matrix.sh` covers the other
# direction: that enabling those features does pull them.
REQSIGN_FORBIDDEN_BY_DEFAULT = {
    "reqsign-azure-storage",
    "reqsign-google",
}


def fail(message: str) -> None:
    raise SystemExit(f"downstream dependency check failed: {message}")


def main() -> None:
    metadata = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
    packages = metadata["packages"]

    opendal = [package for package in packages if package["name"] == "opendal"]
    if len(opendal) != 1:
        fail(f"expected one opendal package, found {len(opendal)}")
    opendal_source = opendal[0].get("source") or ""
    expected_opendal = "git+https://github.com/entrepeneur4lyf/opendal.git?rev="
    if not opendal_source.startswith(expected_opendal) or OPENDAL_REV not in opendal_source:
        fail(f"opendal did not resolve from exact fork commit: {opendal_source}")

    reqsign = {
        package["name"]: package.get("source") or ""
        for package in packages
        if package["name"].startswith("reqsign-")
    }
    leaked = sorted(set(reqsign) & REQSIGN_FORBIDDEN_BY_DEFAULT)
    if leaked:
        fail(
            f"{', '.join(leaked)} resolved for a filesystem-only consumer — "
            "the Azure/GCS feature gating regressed, and rsa "
            "(RUSTSEC-2023-0071) is back in the default graph"
        )
    missing = sorted(REQSIGN_REQUIRED - set(reqsign))
    if missing:
        fail(f"expected Reqsign crates absent: {', '.join(missing)}")
    unknown = sorted(set(reqsign) - REQSIGN_REQUIRED - REQSIGN_FORBIDDEN_BY_DEFAULT)
    if unknown:
        fail(f"unrecognised Reqsign crates resolved: {', '.join(unknown)}")
    for name, source in sorted(reqsign.items()):
        expected = "git+https://github.com/apache/opendal-reqsign.git?rev="
        if not source.startswith(expected) or REQSIGN_REV not in source:
            fail(f"{name} did not resolve from exact official commit: {source}")

    quick_xml = [
        package["version"] for package in packages if package["name"] == "quick-xml"
    ]
    if not quick_xml:
        fail("quick-xml was not resolved")
    for version in quick_xml:
        core = tuple(int(part) for part in version.split("-", 1)[0].split("."))
        if core < (0, 41, 0):
            fail(f"vulnerable quick-xml version resolved: {version}")

    print(f"opendal source={opendal_source}")
    for name, source in sorted(reqsign.items()):
        print(f"{name} source={source}")
    print(f"quick-xml versions={','.join(sorted(quick_xml))}")


if __name__ == "__main__":
    main()
