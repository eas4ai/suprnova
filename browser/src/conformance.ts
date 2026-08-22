import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";

export const FIXTURE_FILES_V1 = [
  "canonical-success.json",
  "canonical-failure.json",
  "snapshot-success.json",
  "snapshot-failure.json",
  "protocol-success.json",
  "protocol-failure.json",
  "response-ordering.json",
  "compatibility.json",
] as const;

export const FIXTURE_FILES_V2 = [
  "protocol-success.json",
  "protocol-failure.json",
  "compatibility.json",
] as const;

export const FIXTURE_FILES_V3 = [
  "compatibility.json",
  "diagnostics.json",
  "directive-grammar.json",
  "island-metadata.json",
  "morph-identity.json",
  "navigation.json",
  "response-application.json",
  "runtime-config.json",
  "scheduling.json",
] as const;

export type FixtureVersion = 1 | 2 | 3;

export const FIXTURE_SETS = [
  { version: 1, files: FIXTURE_FILES_V1 },
  { version: 2, files: FIXTURE_FILES_V2 },
  { version: 3, files: FIXTURE_FILES_V3 },
] as const satisfies readonly {
  readonly version: FixtureVersion;
  readonly files: readonly string[];
}[];

function fixtureSet(version: FixtureVersion): (typeof FIXTURE_SETS)[number] {
  const fixtureSet = FIXTURE_SETS.find((candidate) => candidate.version === version);
  if (fixtureSet === undefined) throw new TypeError("unsupported_fixture_version");
  return fixtureSet;
}

function fixtureDirectory(version: FixtureVersion): URL {
  return new URL(`../../fixtures/v${String(version)}/`, import.meta.url);
}

export async function loadFixtureSet(
  version: FixtureVersion = 1,
): Promise<ReadonlyMap<string, unknown>> {
  const fixture = fixtureSet(version);
  const directory = fixtureDirectory(version);
  const entries = await Promise.all(
    fixture.files.map(async (name) => {
      const text = await readFile(new URL(name, directory), "utf8");
      const value: unknown = JSON.parse(text);
      return [name, value] as const;
    }),
  );
  return new Map(entries);
}

export async function loadFixtureSets(): Promise<
  ReadonlyMap<FixtureVersion, ReadonlyMap<string, unknown>>
> {
  const entries = await Promise.all(
    FIXTURE_SETS.map(async ({ version }) => [version, await loadFixtureSet(version)] as const),
  );
  return new Map(entries);
}

export async function fixtureManifestSha256(version: FixtureVersion = 1): Promise<string> {
  const fixture = fixtureSet(version);
  const directory = fixtureDirectory(version);
  const hash = createHash("sha256");
  for (const name of fixture.files) {
    hash.update(name, "utf8");
    hash.update(new Uint8Array([0]));
    hash.update(await readFile(new URL(name, directory)));
    hash.update(new Uint8Array([0]));
  }
  return hash.digest("hex");
}

export async function expectedFixtureManifestSha256(version: FixtureVersion = 1): Promise<string> {
  return (await readFile(new URL("manifest.sha256", fixtureDirectory(version)), "utf8")).trim();
}
