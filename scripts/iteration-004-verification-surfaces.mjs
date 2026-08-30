import fs from "node:fs";
import path from "node:path";

function filesBelow(directory, predicate) {
  if (!fs.existsSync(directory)) return [];
  return fs
    .readdirSync(directory, { recursive: true, withFileTypes: true })
    .filter((entry) => entry.isFile() && predicate(entry.name))
    .map((entry) => path.join(entry.parentPath, entry.name));
}

function namedFiles(directory, expression) {
  return filesBelow(directory, (name) => expression.test(name));
}

function directFiles(directory, expression) {
  if (!fs.existsSync(directory)) return [];
  return fs
    .readdirSync(directory, { withFileTypes: true })
    .filter((entry) => entry.isFile() && expression.test(entry.name))
    .map((entry) => path.join(directory, entry.name));
}

/**
 * Returns the mechanically owned Iteration 004 verification surface.
 *
 * Test files and the test-support reference host are scanned in full. Product
 * modules are present only when they own inline Iteration 004 tests, and those
 * entries are explicitly restricted to `#[cfg(test)]` regions so legitimate
 * runtime timers are not treated as test synchronization.
 */
export function iteration004VerificationSurfaces(repositoryRoot) {
  const tests = path.join(repositoryRoot, "tests");
  const browserTests = path.join(repositoryRoot, "browser/tests");
  const browserE2e = path.join(repositoryRoot, "browser/e2e");
  const browserHost = path.join(repositoryRoot, "browser/test-host");
  const referenceHost = path.join(
    repositoryRoot,
    "crates/suprnova-live-test-support/src/reference_host",
  );

  // Root integration tests are wholly verification-owned; scanning the full
  // directory prevents a newly named regression from escaping a feature-name
  // allowlist. Compile fixtures are intentionally excluded.
  const rustIntegration = directFiles(tests, /\.rs$/u);
  const rustReferenceHost = [
    ...namedFiles(referenceHost, /\.rs$/u),
    path.join(
      repositoryRoot,
      "crates/suprnova-live-test-support/tests/reference_host.rs",
    ),
  ];
  // Browser unit tests and support modules are verification-owned. The entire
  // tree is safer than inferring feature ownership from a filename prefix.
  const browserUnit = filesBelow(browserTests, (name) => /\.ts$/u.test(name));
  const browserBrowser = [
    "async-artifacts.spec.ts",
    "async-lifecycle.spec.ts",
    "async-updates.spec.ts",
    "csp.spec.ts",
    "iteration-004-accessibility.spec.ts",
    "iteration-004-adversarial.spec.ts",
    "iteration-004-integration.spec.ts",
    "iteration-004-lifecycle.spec.ts",
    "registered-event-authority.spec.ts",
    "uploads.spec.ts",
  ].map((name) => path.join(browserE2e, name));
  const browserE2eSupport = namedFiles(
    path.join(browserE2e, "support"),
    /\.ts$/u,
  );
  const browserReferenceHost = namedFiles(browserHost, /\.(?:mjs|ts)$/u);

  const full = [
    ...rustIntegration,
    ...rustReferenceHost,
    ...browserUnit,
    ...browserBrowser,
    ...browserE2eSupport,
    ...browserReferenceHost,
  ].map((filePath) => ({ filePath, region: "full" }));

  return [
    ...full,
    {
      filePath: path.join(repositoryRoot, "src/upload/provider.rs"),
      region: "rust-cfg-test",
    },
  ].sort((left, right) => left.filePath.localeCompare(right.filePath));
}
