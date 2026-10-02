"""Minimum observation coverage, not package validation rules."""

SCHEMA_CASES = (
    "valid-all-kinds", "unknown-schema", "unknown-catalog-field",
    "duplicate-catalog-key", "unknown-manifest-field", "duplicate-manifest-key",
    "reserved-namespace", "invalid-component-name", "invalid-version",
    "unbounded-compatibility", "incompatible-version", "implicit-prerelease",
    "invalid-license", "unknown-profile", "manifest-identity-mismatch",
    "invalid-file-role", "file-order-mismatch", "missing-template",
    "invalid-utf8", "missing-dependency", "duplicate-dependency", "dependency-cycle",
    "duplicate-element", "reserved-element", "element-without-script",
    "invalid-rust-declaration", "duplicate-view-entry", "unknown-schema-kind",
    "schema-extra-field", "schema-duplicate-key", "schema-external-reference",
)

FILESYSTEM_CASES = (
    "valid-package", "source-traversal", "absolute-path", "source-link",
    "link-swap", "special-file", "missing-file", "undeclared-file",
    "undeclared-directory", "changed-payload", "duplicate-file",
)

LIMITS = (
    "manifest-bytes", "payload-bytes", "package-bytes", "components",
    "payload-count", "element-count", "dependency-count", "json-depth",
)
LIMIT_CASES = tuple(f"{name}-{edge}" for name in LIMITS for edge in ("at", "over"))
LEGACY_CASES = (
    "official-install", "third-party-install", "edited-file-preserved",
    "repeat-install", "legacy-remains-legacy", "schema1-explicit",
)

MATRICES = {
    "LCT-002": SCHEMA_CASES,
    "LCT-003": FILESYSTEM_CASES,
    "LCT-004": LIMIT_CASES,
    "LCT-005": SCHEMA_CASES,
    "LCT-006": LEGACY_CASES,
}
POSITIVE_CASES = {"valid-all-kinds", "valid-package", *LEGACY_CASES,
                  *(f"{name}-at" for name in LIMITS)}
