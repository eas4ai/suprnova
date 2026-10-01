import { spawn } from "node:child_process";
import { mkdir, mkdtemp, readFile, rm } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { atomicWriteEvidence } from "./run-upload-budget.mjs";

const MAX_PROCESS_RESULT_BYTES = 1_048_576;
const PROCESS_WATCHDOG_MILLISECONDS = 30 * 60 * 1_000;

function parseArguments(argv) {
  const options = { baseline: null, cpuSet: "0-7", destination: null, profile: null };
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    const value = argv[index + 1];
    if (
      !["--baseline", "--cpu-set", "--output", "--profile"].includes(argument) ||
      value === undefined
    ) {
      throw new Error("usage");
    }
    index += 1;
    if (argument === "--baseline") options.baseline = resolve(value);
    else if (argument === "--cpu-set") options.cpuSet = value;
    else if (argument === "--output") options.destination = resolve(value);
    else options.profile = value;
  }
  if (
    options.destination === null ||
    (options.profile !== "qualified" && options.profile !== "reduced") ||
    !/^\d+(?:-\d+)?(?:,\d+(?:-\d+)?)*$/u.test(options.cpuSet)
  ) {
    throw new Error("usage");
  }
  return Object.freeze(options);
}

async function boundedProcessResult(path, expectedRunIndex) {
  const bytes = await readFile(path);
  if (bytes.byteLength < 2 || bytes.byteLength > MAX_PROCESS_RESULT_BYTES) {
    throw new Error("server_process_evidence_invalid");
  }
  let value;
  try {
    value = JSON.parse(bytes.toString("utf8"));
  } catch {
    throw new Error("server_process_evidence_invalid");
  }
  if (
    typeof value !== "object" ||
    value === null ||
    Array.isArray(value) ||
    value.runIndex !== expectedRunIndex ||
    !Number.isSafeInteger(value.processId) ||
    value.processId <= 0
  ) {
    throw new Error("server_process_evidence_invalid");
  }
  return value;
}

async function productionRunProcess({ cpuSet, profile, resultPath, runIndex }) {
  const arguments_ = [
    "env",
    "CARGO_INCREMENTAL=0",
    `SUPRNOVA_LIVE_UPLOAD_SERVER_RESULT=${resultPath}`,
    `SUPRNOVA_LIVE_UPLOAD_SERVER_RUN_INDEX=${String(runIndex)}`,
  ];
  if (profile === "qualified") arguments_.push("SUPRNOVA_LIVE_REQUIRE_S1=1");
  arguments_.push("taskset", "-c", cpuSet, "cargo", "bench", "--bench", "upload_framework_budget");
  await new Promise((resolveRun, rejectRun) => {
    const controller = new AbortController();
    const watchdog = setTimeout(() => controller.abort(), PROCESS_WATCHDOG_MILLISECONDS);
    let settled = false;
    const settle = (operation) => {
      if (settled) return;
      settled = true;
      clearTimeout(watchdog);
      operation();
    };
    const child = spawn("rtk", arguments_, {
      cwd: resolve(dirname(fileURLToPath(import.meta.url)), "../.."),
      signal: controller.signal,
      stdio: "inherit",
    });
    let spawnError;
    child.once("error", (error) => {
      spawnError = error;
    });
    child.once("close", (code, signal) => {
      settle(() => {
        if (code === 0 && spawnError === undefined) resolveRun();
        else if (spawnError !== undefined) rejectRun(spawnError);
        else rejectRun(new Error(`server_process_failed:${String(code)}:${String(signal)}`));
      });
    });
  });
}

export async function collectUploadServerRuns({
  baseline = null,
  cpuSet = "0-7",
  destination,
  profile,
  runProcess = productionRunProcess,
}) {
  const runCount = profile === "qualified" ? 3 : profile === "reduced" ? 1 : 0;
  if (runCount === 0) throw new Error("profile_invalid");
  await mkdir(dirname(destination), { recursive: true });
  const temporaryDirectory = await mkdtemp(join(dirname(destination), ".upload-server-runs-"));
  try {
    const processRuns = [];
    for (let runIndex = 1; runIndex <= runCount; runIndex += 1) {
      const resultPath = join(temporaryDirectory, `run-${String(runIndex)}.json`);
      await runProcess({ cpuSet, profile, resultPath, runIndex });
      processRuns.push(await boundedProcessResult(resultPath, runIndex));
    }
    const processIds = processRuns.map(({ processId }) => processId);
    if (new Set(processIds).size !== processIds.length) {
      throw new Error("server_process_identity_reused");
    }
    await atomicWriteEvidence(
      destination,
      `${JSON.stringify({ processRuns, schemaVersion: 1 }, null, 2)}\n`,
      baseline,
    );
  } finally {
    await rm(temporaryDirectory, { force: true, recursive: true });
  }
}

async function main() {
  try {
    const options = parseArguments(process.argv.slice(2));
    await collectUploadServerRuns(options);
  } catch (error) {
    process.stderr.write(
      `U4/16 upload server process runner failed: ${error instanceof Error ? error.message : "internal"}\n`,
    );
    process.exitCode = 1;
  }
}

if (process.argv[1] !== undefined && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  await main();
}
