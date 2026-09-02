#!/usr/bin/env node

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const scriptDirectory = path.dirname(fileURLToPath(import.meta.url));
const repositoryRoot = path.resolve(scriptDirectory, "..");
const implementationDirectory = path.join(
  repositoryRoot,
  "docs",
  "implementation",
);
const files = [
  "README.md",
  ...fs
    .readdirSync(implementationDirectory)
    .filter((name) => name.endsWith(".md"))
    .sort()
    .map((name) => path.join("docs", "implementation", name)),
];
const failures = [];
const semanticOnly = process.argv.slice(2).includes("--semantic-only");
const requiredHeadings = new Map([
  [
    "docs/implementation/uploads.md",
    [
      "Handle and grant",
      "Provider modes",
      "Quarantine and scanning",
      "Finalization and compensation",
      "Current-document resume",
      "Cleanup",
    ],
  ],
  [
    "docs/implementation/async-updates.md",
    [
      "Event schemas",
      "Subscription authorization",
      "Polling and push modes",
      "Continuity",
      "Degraded freshness",
      "Backpressure",
    ],
  ],
  [
    "docs/implementation/iteration-004-operations.md",
    [
      "Artifacts",
      "Limits",
      "Observability",
      "Benchmarks",
      "Reference-host boundary",
      "Suprnova integration boundary",
    ],
  ],
]);
const semanticRequirements = new Map([
  [
    "README.md",
    [
      {
        id: "readme_uploads_link",
        pattern: /\[Uploads\]\(docs\/implementation\/uploads\.md\)/u,
      },
      {
        id: "readme_async_link",
        pattern:
          /\[asynchronous updates\]\(docs\/implementation\/async-updates\.md\)/u,
      },
      {
        id: "readme_operations_link",
        pattern:
          /\[Iteration 004 operations\]\(docs\/implementation\/iteration-004-operations\.md\)/u,
      },
    ],
  ],
  [
    "docs/implementation/benchmarking.md",
    [
      {
        id: "benchmark_snapshot_integrated_crate_root",
        pattern:
          /Run a local exploratory measurement from the integrated crate root:[\s\S]{0,150}rtk env CARGO_INCREMENTAL=0 scripts\/run-snapshot-budget\.sh/u,
      },
      {
        id: "benchmark_action_integrated_crate_root",
        pattern:
          /Run it from the integrated crate root:[\s\S]{0,150}rtk env CARGO_INCREMENTAL=0 scripts\/run-action-budget\.sh/u,
      },
    ],
  ],
  [
    "docs/implementation/browser-assets.md",
    [
      {
        id: "browser_assets_current_host_ownership",
        pattern:
          /Iteration 005's Suprnova host adapter owns asset authorization, compression, CDN purge, and framework route registration\.[\s\S]{0,300}not complete until/u,
      },
    ],
  ],
  [
    "docs/implementation/host-adapter-contract.md",
    [
      {
        id: "host_adapter_current_ownership",
        pattern:
          /Iteration 005's Suprnova adapter must own truthful normalization and enforcement[\s\S]{0,900}not claimed complete until/u,
      },
    ],
  ],
  [
    "docs/implementation/uploads.md",
    [
      {
        id: "upload_handle_is_not_grant",
        pattern:
          /`UploadHandle` is[\s\S]{0,500}not proof[\s\S]{0,500}`TransferGrant` is secret bearer authority/u,
      },
      {
        id: "upload_grant_secret_hygiene",
        pattern: /A grant is never persisted, rendered, or logged/u,
      },
      {
        id: "upload_provider_modes",
        pattern: /In reverse-proxy mode[\s\S]{0,2000}In direct-provider mode/u,
      },
      {
        id: "upload_quarantine_media_scanner",
        pattern:
          /temporary quarantine data[\s\S]{0,3000}media parsers[\s\S]{0,1200}`UploadScanner`/u,
      },
      {
        id: "upload_prepare_failure",
        pattern:
          /A prepare error propagates while the upload remains `Finalizing`; it does not call `compensate`\./u,
      },
      {
        id: "upload_compensation_scope",
        pattern:
          /Compensation is attempted only for an invalid prepared result or a commit failure\./u,
      },
      {
        id: "upload_reacquire_application_route",
        pattern: /authenticated\s+application route outside `\/__live\/`/u,
      },
      {
        id: "upload_cleanup_lifecycle",
        pattern:
          /Cancel is a conditional idempotent lifecycle transition[\s\S]{0,2000}bounded cleanup worker/u,
      },
    ],
  ],
  [
    "docs/implementation/async-updates.md",
    [
      {
        id: "async_typed_event_schema",
        pattern: /Events are Rust types[\s\S]{0,500}closed payload contract/u,
      },
      {
        id: "async_signed_authorization_exact_origin",
        pattern:
          /signs it under[\s\S]{0,2500}exact configured or allowlisted origin match/u,
      },
      {
        id: "async_builtin_websocket_cookie_only",
        pattern:
          /The built-in `BrowserWebSocketAdapter` accepts only `session_cookie` authorization\.[\s\S]{0,700}Fetch-based SSE and polling may use bearer authorization/u,
      },
      {
        id: "async_custom_websocket_cross_origin_credentials",
        pattern:
          /A custom bearer-authorized or cross-origin WebSocket transport requires an explicit non-wildcard Origin allowlist and separate non-cookie credentials\.[\s\S]{0,500}Unapproved cross-site origins and every attempt to use cookie authority cross-site fail closed before upgrade\./u,
      },
      {
        id: "async_multiplexed_document_transport",
        pattern:
          /one physical[\s\S]{0,300}transport[\s\S]{0,700}logical memberships/u,
      },
      {
        id: "async_poll_push_hybrid",
        pattern:
          /Polling is an ordinary fresh-render request[\s\S]{0,2200}`live:stream\.push-only/u,
      },
      {
        id: "async_replay_truthful_prefix",
        pattern:
          /Replay prevalidation and admission are atomic for the complete transcript\.[\s\S]{0,500}Dispatch then commits each successful event in order; if a later dispatch fails, recovery reports and preserves the truthful committed prefix/u,
      },
      {
        id: "async_membership_local_auth_loss",
        pattern:
          /Authorization loss removes that membership only; it does not tear down healthy\s+siblings/u,
      },
      {
        id: "async_degraded_fresh_render",
        pattern: /`degraded` means[\s\S]{0,1200}fresh render proves/u,
      },
      {
        id: "async_document_backpressure",
        pattern: /bounded to 64 unapplied envelopes and 256 KiB/u,
      },
      {
        id: "async_fanout_replay_distinction",
        pattern:
          /effective end-to-end event fanout ceiling is 256[\s\S]{0,600}replay transcript limit is 1,024/u,
      },
      {
        id: "async_freeze_offline_lifecycle",
        pattern: /document freeze[\s\S]{0,700}Offline state pauses/u,
      },
    ],
  ],
  [
    "docs/implementation/iteration-004-operations.md",
    [
      {
        id: "operations_exact_artifacts",
        pattern:
          /`suprnova-live\.esm\.js`[\s\S]{0,500}`suprnova-live\.classic\.js`[\s\S]{0,500}`suprnova-live\.stimulus\.esm\.js`[\s\S]{0,500}`suprnova-live\.stimulus\.classic\.js`[\s\S]{0,500}`suprnova-live\.uploads\.esm\.js`[\s\S]{0,500}`suprnova-live\.uploads\.classic\.js`[\s\S]{0,500}`suprnova-live\.async\.esm\.js`[\s\S]{0,500}`suprnova-live\.async\.classic\.js`[\s\S]{0,800}Choose ESM or classic/u,
      },
      {
        id: "operations_fixture_protocol_versions",
        pattern:
          /`fixtures\/v4\/`[\s\S]{0,300}Upload protocol\s+v1[\s\S]{0,300}async envelope\/subscription protocol v1/u,
      },
      {
        id: "operations_optional_selection_csp",
        pattern:
          /Trusted checked render metadata\s+selects optional roles[\s\S]{0,1800}`script-src\s+'self'`/u,
      },
      {
        id: "operations_limits_observability",
        pattern:
          /The engine rejects unbounded configuration[\s\S]{0,3500}## Observability/u,
      },
      {
        id: "operations_qualified_baseline",
        pattern: /`qualifiedBaseline` is `null`/u,
      },
      {
        id: "operations_named_workloads",
        pattern: /`U4\/16`[\s\S]{0,1800}`E100\/1K`[\s\S]{0,1400}`R100`/u,
      },
      {
        id: "operations_conformance_boundary",
        pattern:
          /The Rust reference host, Node static host, direct-provider bridge, fault controls, and benchmark harnesses are conformance-only test tools, not production administration APIs\.[\s\S]{0,300}They are neither Suprnova application integration nor vendor integration\./u,
      },
      {
        id: "operations_current_suprnova_ownership",
        pattern:
          /Suprnova application integration owns routes, authentication, session, configuration, provider, scanner, storage, and broadcast wiring\.[\s\S]{0,400}Iteration 005 must implement and prove that ownership through framework tests/u,
      },
      {
        id: "operations_artifact_size_reported_not_budgeted",
        pattern:
          /Artifact size is reported, not budgeted[\s\S]{0,300}no\s+artifact has a cap or drift rule/u,
      },
      {
        id: "operations_fanout_replay_distinction",
        pattern:
          /effective end-to-end event fanout ceiling is 256[\s\S]{0,600}replay transcript limit is 1,024/u,
      },
      {
        id: "operations_warning_policy",
        pattern:
          /Clippy warnings are reviewed; the gate does not blanket-deny warnings/u,
      },
    ],
  ],
]);

function semanticFailures(relativeFile, text) {
  const requirements = semanticRequirements.get(relativeFile) ?? [];
  const normalized = text.replace(/\s+/gu, " ");
  return requirements
    .filter(({ pattern }) => !pattern.test(normalized))
    .map(({ id }) => `${relativeFile}: missing semantic contract: ${id}`);
}

function mutationSelfTest(documents) {
  const cases = [
    {
      file: "docs/implementation/uploads.md",
      pattern: /it does not call\s+`compensate`/u,
      replacement: "it calls `compensate`",
      requirement: "upload_prepare_failure",
    },
    {
      file: "docs/implementation/async-updates.md",
      pattern: /truthful committed prefix/u,
      replacement: "all-or-nothing dispatch",
      requirement: "async_replay_truthful_prefix",
    },
    {
      file: "docs/implementation/async-updates.md",
      pattern:
        /The built-in `BrowserWebSocketAdapter`\s+accepts only `session_cookie` authorization\./u,
      replacement:
        "The built-in `BrowserWebSocketAdapter` accepts `session_cookie` or bearer authorization.",
      requirement: "async_builtin_websocket_cookie_only",
    },
    {
      file: "docs/implementation/async-updates.md",
      pattern: /separate non-cookie credentials/u,
      replacement: "cross-site cookie authority",
      requirement: "async_custom_websocket_cross_origin_credentials",
    },
    {
      file: "docs/implementation/iteration-004-operations.md",
      pattern: /effective end-to-end\s+event fanout ceiling is 256/u,
      replacement: "effective end-to-end event fanout ceiling is 1,024",
      requirement: "operations_fanout_replay_distinction",
    },
    {
      file: "docs/implementation/iteration-004-operations.md",
      pattern: /not production administration APIs/u,
      replacement: "production administration APIs",
      requirement: "operations_conformance_boundary",
    },
    {
      file: "docs/implementation/iteration-004-operations.md",
      pattern:
        /They are\s+neither Suprnova application integration nor vendor integration\./u,
      replacement:
        "They are both Suprnova application integration and vendor integration.",
      requirement: "operations_conformance_boundary",
    },
    {
      file: "docs/implementation/iteration-004-operations.md",
      pattern: /Suprnova application integration owns routes/u,
      replacement:
        "Historical planning said a future Suprnova integration might own framework wiring.",
      requirement: "operations_current_suprnova_ownership",
    },
    {
      file: "docs/implementation/browser-assets.md",
      pattern:
        /Iteration 005's Suprnova host adapter owns asset authorization/u,
      replacement:
        "An eventual Suprnova host adapter may own asset authorization",
      requirement: "browser_assets_current_host_ownership",
    },
    {
      file: "docs/implementation/host-adapter-contract.md",
      pattern:
        /Iteration 005's Suprnova adapter must own truthful normalization and enforcement/u,
      replacement:
        "An eventual Suprnova adapter may own truthful normalization and enforcement",
      requirement: "host_adapter_current_ownership",
    },
    {
      file: "docs/implementation/benchmarking.md",
      pattern:
        /Run a local exploratory measurement from the integrated crate root:/u,
      replacement:
        "Run a local exploratory measurement from the repository root:",
      requirement: "benchmark_snapshot_integrated_crate_root",
    },
    {
      file: "docs/implementation/benchmarking.md",
      pattern: /Run it from the integrated crate root:/u,
      replacement: "Run it from the repository root:",
      requirement: "benchmark_action_integrated_crate_root",
    },
    {
      file: "README.md",
      pattern: /docs\/implementation\/uploads\.md/u,
      replacement: "docs/implementation/upload.md",
      requirement: "readme_uploads_link",
    },
  ];
  const mutationFailures = [];
  for (const mutation of cases) {
    const original = documents.get(mutation.file);
    if (original === undefined || !mutation.pattern.test(original)) {
      mutationFailures.push(
        `semantic mutation anchor missing: ${mutation.requirement}`,
      );
      continue;
    }
    const mutated = original.replace(mutation.pattern, mutation.replacement);
    const detected = semanticFailures(mutation.file, mutated).some((failure) =>
      failure.endsWith(mutation.requirement),
    );
    if (!detected)
      mutationFailures.push(
        `semantic mutation escaped: ${mutation.requirement}`,
      );
  }
  return { cases: cases.length, failures: mutationFailures };
}

const semanticDocuments = new Map();
for (const relativeFile of semanticRequirements.keys()) {
  const fullPath = path.join(repositoryRoot, relativeFile);
  if (!fs.existsSync(fullPath)) {
    failures.push(`${relativeFile}: required semantic document is missing`);
    continue;
  }
  const text = fs.readFileSync(fullPath, "utf8");
  semanticDocuments.set(relativeFile, text);
  failures.push(...semanticFailures(relativeFile, text));
}

if (failures.length === 0) {
  const mutation = mutationSelfTest(semanticDocuments);
  failures.push(...mutation.failures);
  if (mutation.failures.length === 0) {
    console.log(
      `implementation-doc semantic mutation self-test ok cases=${mutation.cases}`,
    );
  }
}

for (const [relativeFile, headings] of semanticOnly ? [] : requiredHeadings) {
  const fullPath = path.join(repositoryRoot, relativeFile);
  if (!fs.existsSync(fullPath)) {
    failures.push(
      `${relativeFile}: required implementation document is missing`,
    );
    continue;
  }

  const text = fs.readFileSync(fullPath, "utf8");
  for (const heading of headings) {
    if (!text.split("\n").includes(`## ${heading}`)) {
      failures.push(`${relativeFile}: missing exact heading: ## ${heading}`);
    }
  }
}

for (const relativeFile of semanticOnly ? [] : files) {
  const fullPath = path.join(repositoryRoot, relativeFile);
  const text = fs.readFileSync(fullPath, "utf8");

  if (text.includes("\r")) {
    failures.push(`${relativeFile}: contains a carriage-return character`);
  }
  if (!text.endsWith("\n") || text.endsWith("\n\n")) {
    failures.push(`${relativeFile}: must end with exactly one newline`);
  }
  if (/^[^\n]*[ \t]+$/m.test(text)) {
    failures.push(`${relativeFile}: contains trailing whitespace`);
  }
  if (/\b(?:TODO|TBD|PLACEHOLDER)\b/u.test(text)) {
    failures.push(`${relativeFile}: contains an unresolved placeholder marker`);
  }
  if (text.includes("-D warnings")) {
    failures.push(
      `${relativeFile}: recommends forbidden blanket warning denial`,
    );
  }

  for (const match of text.matchAll(/!?\[[^\]]*\]\(([^)]+)\)/g)) {
    let target = match[1].trim();
    if (target.startsWith("<") && target.endsWith(">")) {
      target = target.slice(1, -1);
    }
    if (
      target === "" ||
      target.startsWith("#") ||
      target.startsWith("/") ||
      /^[a-z][a-z0-9+.-]*:/i.test(target)
    ) {
      continue;
    }

    let localTarget;
    try {
      localTarget = decodeURIComponent(target.split(/[?#]/, 1)[0]);
    } catch {
      failures.push(`${relativeFile}: malformed relative link: ${target}`);
      continue;
    }
    const resolved = path.resolve(path.dirname(fullPath), localTarget);
    if (!fs.existsSync(resolved)) {
      failures.push(`${relativeFile}: broken relative link: ${target}`);
    }
  }
}

if (failures.length > 0) {
  console.error(
    `implementation-doc-check failed with ${failures.length} issue(s):`,
  );
  for (const failure of failures) {
    console.error(`- ${failure}`);
  }
  process.exit(1);
}

console.log(
  semanticOnly
    ? `implementation-doc-semantics ok files=${semanticRequirements.size}`
    : `implementation-doc-check ok files=${files.length}`,
);
