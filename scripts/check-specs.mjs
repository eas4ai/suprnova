#!/usr/bin/env node

import fs from "node:fs"
import path from "node:path"
import { spawnSync } from "node:child_process"
import { fileURLToPath } from "node:url"

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repositoryRoot = path.resolve(scriptDir, "..")
const specDirectory = path.join(
  repositoryRoot,
  "docs",
  "specs",
  "suprnova-live",
)

const domainFiles = Array.from({ length: 26 }, (_, number) => {
  const prefix = String(number).padStart(2, "0")
  const match = fs
    .readdirSync(specDirectory)
    .find((name) => name.startsWith(`${prefix}-`) && name.endsWith(".md"))

  if (!match) {
    throw new Error(`missing numbered specification with prefix ${prefix}`)
  }

  return match
})

const companionFiles = ["conventions.md", "glossary.md", "ux.md"]
const requiredFiles = [...domainFiles, ...companionFiles].sort()
const actualFiles = fs
  .readdirSync(specDirectory, { withFileTypes: true })
  .filter((entry) => entry.isFile() && entry.name.endsWith(".md"))
  .map((entry) => entry.name)
  .sort()

const failures = []
let capabilityCount = 0
let acceptanceBlockCount = 0
let uxFlowCount = 0
let archiveFileCount = null

function fail(file, message) {
  failures.push(`${file}: ${message}`)
}

function occurrences(text, pattern) {
  return [...text.matchAll(pattern)].length
}

function assertSection(file, text, heading) {
  if (!text.includes(`\n## ${heading}\n`)) {
    fail(file, `missing required section "## ${heading}"`)
  }
}

if (requiredFiles.join("\n") !== actualFiles.join("\n")) {
  const missing = requiredFiles.filter((file) => !actualFiles.includes(file))
  const unexpected = actualFiles.filter((file) => !requiredFiles.includes(file))

  if (missing.length > 0) {
    failures.push(`spec set: missing files: ${missing.join(", ")}`)
  }
  if (unexpected.length > 0) {
    failures.push(`spec set: unexpected files: ${unexpected.join(", ")}`)
  }
}

const fileContents = new Map()

for (const file of actualFiles) {
  const fullPath = path.join(specDirectory, file)
  const text = fs.readFileSync(fullPath, "utf8")
  fileContents.set(file, text)

  if (text.includes("\r")) {
    fail(file, "contains CRLF or bare carriage-return characters")
  }
  if (!text.endsWith("\n")) {
    fail(file, "must end with one newline")
  }
  if (text.endsWith("\n\n")) {
    fail(file, "contains a blank line at EOF")
  }
  if (/^[^\n]*[ \t]+$/m.test(text)) {
    fail(file, "contains trailing whitespace")
  }
  if (text.includes("—")) {
    fail(file, "contains an em dash; published Suprnova prose uses hyphens")
  }
  if (
    /_To be completed|Completed in Stage 5 after|date of last edit|Project Name --|One or two paragraphs:/.test(
      text,
    )
  ) {
    fail(file, "contains template or stage placeholder prose")
  }
  if (file !== "glossary.md") {
    if (!/^Status: (Normative|Normative design specification)$/m.test(text)) {
      fail(file, "has no recognized normative Status line")
    }
    const lastRevisedMatch = text.match(
      /^Last revised: (\d{4}-\d{2}-\d{2})$/m,
    )
    if (!lastRevisedMatch) {
      fail(file, "has no ISO Last revised date")
    }

    const decisionHeadingCount = occurrences(
      text,
      /^## Decisions and revisions$/gm,
    )
    if (decisionHeadingCount !== 1) {
      fail(
        file,
        `must contain exactly one Decisions and revisions section; found ${decisionHeadingCount}`,
      )
    }

    const decisionDates = [
      ...text.matchAll(/^- (\d{4}-\d{2}-\d{2}) -- /gm),
    ].map((match) => match[1])
    if (decisionDates.length === 0) {
      fail(file, "contains no dated decision entry")
    } else if (
      lastRevisedMatch &&
      lastRevisedMatch[1] < decisionDates.reduce((a, b) => (a > b ? a : b))
    ) {
      fail(file, "Last revised is older than the newest decision entry")
    }
    for (let index = 1; index < decisionDates.length; index += 1) {
      if (decisionDates[index] > decisionDates[index - 1]) {
        fail(file, "decision entries are not newest first")
        break
      }
    }
  }

  for (const match of text.matchAll(/!?\[[^\]]*\]\(([^)]+)\)/g)) {
    let target = match[1].trim()
    if (target.startsWith("<") && target.endsWith(">")) {
      target = target.slice(1, -1)
    }
    if (
      target === "" ||
      target.startsWith("#") ||
      target.startsWith("/") ||
      /^[a-z][a-z0-9+.-]*:/i.test(target)
    ) {
      continue
    }

    const localTarget = decodeURIComponent(target.split(/[?#]/, 1)[0])
    const resolved = path.resolve(path.dirname(fullPath), localTarget)
    if (!fs.existsSync(resolved)) {
      fail(file, `broken relative link: ${target}`)
    }
  }
}

const archivePath = path.join(
  repositoryRoot,
  "docs",
  "specs",
  "suprnova-live.zip",
)
if (fs.existsSync(archivePath)) {
  const listing = spawnSync("unzip", ["-Z1", archivePath], {
    encoding: "utf8",
    maxBuffer: 1024 * 1024,
  })

  if (listing.error || listing.status !== 0) {
    failures.push(
      `spec archive: cannot list archive: ${listing.error?.message ?? listing.stderr.trim()}`,
    )
  } else {
    const actualEntries = listing.stdout.trim().split("\n").sort()
    const expectedEntries = requiredFiles
      .map((file) => `suprnova-live/${file}`)
      .sort()
    const actualDocumentEntries = actualEntries.filter((entry) =>
      entry.endsWith(".md"),
    )

    if (actualDocumentEntries.join("\n") !== expectedEntries.join("\n")) {
      const missing = expectedEntries.filter(
        (entry) => !actualDocumentEntries.includes(entry),
      )
      const unexpected = actualEntries.filter(
        (entry) => !entry.endsWith("/") && !expectedEntries.includes(entry),
      )
      if (missing.length > 0) {
        failures.push(`spec archive: missing entries: ${missing.join(", ")}`)
      }
      if (unexpected.length > 0) {
        failures.push(
          `spec archive: unexpected entries: ${unexpected.join(", ")}`,
        )
      }
    }

    for (const file of requiredFiles) {
      const archived = spawnSync(
        "unzip",
        ["-p", archivePath, `suprnova-live/${file}`],
        { encoding: "utf8", maxBuffer: 4 * 1024 * 1024 },
      )
      if (archived.error || archived.status !== 0) {
        failures.push(
          `spec archive: cannot read ${file}: ${archived.error?.message ?? archived.stderr.trim()}`,
        )
        continue
      }
      if (archived.stdout !== fileContents.get(file)) {
        failures.push(`spec archive: ${file} differs from the source document`)
      }
    }

    archiveFileCount = actualDocumentEntries.length
  }
}

const overview = fileContents.get("00-overview.md") ?? ""
for (const section of [
  "Purpose",
  "Design principles",
  "System architecture",
  "Cross-cutting requirements",
  "Spec map",
  "Supported and excluded scope",
  "Revision policy",
  "System completion criteria",
  "Decisions and revisions",
]) {
  assertSection("00-overview.md", overview, section)
}

for (const file of domainFiles.slice(1)) {
  const text = fileContents.get(file) ?? ""
  const expectedPrefix = file.slice(0, 2)

  if (!new RegExp(`^# Suprnova Live -- ${expectedPrefix} `, "m").test(text)) {
    fail(file, `title does not match numbered prefix ${expectedPrefix}`)
  }
  for (const section of [
    "Scope",
    "Capabilities",
    "Acceptance criteria",
    "Decisions and revisions",
  ]) {
    assertSection(file, text, section)
  }

  const capabilitiesStart = text.indexOf("\n## Capabilities\n")
  const acceptanceStart = text.indexOf("\n## Acceptance criteria\n")
  if (capabilitiesStart === -1 || acceptanceStart <= capabilitiesStart) {
    fail(file, "Capabilities must precede domain Acceptance criteria")
    continue
  }

  const capabilities = text.slice(capabilitiesStart, acceptanceStart)
  const matches = [...capabilities.matchAll(/^### (.+)$/gm)]
  if (matches.length === 0) {
    fail(file, "contains no capability subsections")
  }

  capabilityCount += matches.length
  for (let index = 0; index < matches.length; index += 1) {
    const start = matches[index].index
    const end = matches[index + 1]?.index ?? capabilities.length
    const block = capabilities.slice(start, end)
    const acceptanceCount = occurrences(block, /^Acceptance criteria:$/gm)
    const flowCount = occurrences(block, /^UX flow:$/gm)

    acceptanceBlockCount += acceptanceCount
    uxFlowCount += flowCount

    if (acceptanceCount !== 1) {
      fail(
        file,
        `capability "${matches[index][1]}" has ${acceptanceCount} acceptance blocks`,
      )
    }
    if (flowCount !== 1) {
      fail(
        file,
        `capability "${matches[index][1]}" has ${flowCount} UX flows`,
      )
    }
  }
}

for (const file of domainFiles.slice(1)) {
  const marker = `| \`${file}\` |`
  const count = overview.split(marker).length - 1
  if (count !== 1) {
    fail(
      "00-overview.md",
      `spec map must contain ${file} exactly once; found ${count}`,
    )
  }
}

const glossary = fileContents.get("glossary.md") ?? ""
const glossaryTermCount = occurrences(glossary, /^\*\*[^*]+\*\*:/gm)
const glossaryAvoidCount = occurrences(glossary, /^_Avoid_:/gm)
if (glossaryTermCount !== glossaryAvoidCount) {
  fail(
    "glossary.md",
    `${glossaryTermCount} terms do not match ${glossaryAvoidCount} Avoid lines`,
  )
}

const ux = fileContents.get("ux.md") ?? ""
for (const section of [
  "Interaction model",
  "User journeys",
  "Surface map",
  "Decision points and branching",
  "Error and recovery flows",
  "Platform divergences",
  "Decisions and revisions",
]) {
  assertSection("ux.md", ux, section)
}

const conventions = fileContents.get("conventions.md") ?? ""
for (const section of [
  "Implementation standards",
  "Naming and organization",
  "Verification commands",
  "Decisions and revisions",
]) {
  assertSection("conventions.md", conventions, section)
}

if (failures.length > 0) {
  console.error(`spec-check failed with ${failures.length} issue(s):`)
  for (const failure of failures) {
    console.error(`- ${failure}`)
  }
  process.exit(1)
}

console.log(
  [
    "spec-check ok",
    `files=${actualFiles.length}`,
    `domains=${domainFiles.length - 1}`,
    `capabilities=${capabilityCount}`,
    `acceptance_blocks=${acceptanceBlockCount}`,
    `ux_flows=${uxFlowCount}`,
    `glossary_terms=${glossaryTermCount}`,
    `archive_files=${archiveFileCount ?? "absent"}`,
  ].join(" "),
)
