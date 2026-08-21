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

const fixtureDirectory = new URL("../../fixtures/v1/", import.meta.url);

export async function loadFixtureSet(): Promise<ReadonlyMap<string, unknown>> {
  const entries = await Promise.all(
    FIXTURE_FILES_V1.map(async (name) => {
      const text = await readFile(new URL(name, fixtureDirectory), "utf8");
      const value: unknown = JSON.parse(text);
      return [name, value] as const;
    }),
  );
  return new Map(entries);
}

export async function fixtureManifestSha256(): Promise<string> {
  const hash = createHash("sha256");
  for (const name of FIXTURE_FILES_V1) {
    hash.update(name, "utf8");
    hash.update(new Uint8Array([0]));
    hash.update(await readFile(new URL(name, fixtureDirectory)));
    hash.update(new Uint8Array([0]));
  }
  return hash.digest("hex");
}

export async function expectedFixtureManifestSha256(): Promise<string> {
  return (await readFile(new URL("manifest.sha256", fixtureDirectory), "utf8")).trim();
}
