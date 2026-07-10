#!/usr/bin/env python3
"""Verify the isolated consumer resolves the pinned storage security graph."""

from __future__ import annotations

import json
import sys
from pathlib import Path


OPENDAL_REV = "88717391eb72c9839d3f8e79fccad9f22fc3a1b4"
REQSIGN_REV = "b49cd2996b9d2d9944e84481f8835ff55b188b97"
REQSIGN_PACKAGES = {
    "reqsign-aws-v4",
    "reqsign-azure-storage",
    "reqsign-core",
    "reqsign-file-read-tokio",
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
    if set(reqsign) != REQSIGN_PACKAGES:
        fail(f"unexpected Reqsign package set: {sorted(reqsign)}")
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
