import { randomUUID } from "node:crypto";
import { mkdir, readFile, rename, rm, stat, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

export const PRODUCTION_BUILD_HOOK_TIMEOUT_MS = 30_000;
const LOCK_ACQUISITION_TIMEOUT_MS = 25_000;
const INCOMPLETE_LOCK_GRACE_MS = 1_000;
const LOCK_DIRECTORY = join(tmpdir(), "suprnova-live-production-build-v1.lock");

interface LockOwner {
  readonly createdAt: number;
  readonly pid: number;
  readonly token: string;
}

function errorCode(error: unknown): string | null {
  return error instanceof Error && "code" in error && typeof error.code === "string"
    ? error.code
    : null;
}

function processAlive(pid: number): boolean {
  try {
    process.kill(pid, 0);
    return true;
  } catch (error: unknown) {
    return errorCode(error) !== "ESRCH";
  }
}

async function staleOwner(lockDirectory: string): Promise<boolean> {
  const lockOwner = join(lockDirectory, "owner.json");
  try {
    const parsed = JSON.parse(await readFile(lockOwner, "utf8")) as Partial<LockOwner>;
    return (
      !Number.isSafeInteger(parsed.pid) ||
      typeof parsed.pid !== "number" ||
      parsed.pid <= 0 ||
      !Number.isSafeInteger(parsed.createdAt) ||
      typeof parsed.createdAt !== "number" ||
      parsed.createdAt <= 0 ||
      typeof parsed.token !== "string" ||
      parsed.token.length < 32 ||
      parsed.token.length > 64 ||
      !processAlive(parsed.pid)
    );
  } catch (error: unknown) {
    if (errorCode(error) !== "ENOENT") return true;
    try {
      const metadata = await stat(lockDirectory);
      return Date.now() - metadata.mtimeMs >= INCOMPLETE_LOCK_GRACE_MS;
    } catch {
      return false;
    }
  }
}

async function retireStaleLock(lockDirectory: string): Promise<void> {
  if (!(await staleOwner(lockDirectory))) return;
  const retired = `${lockDirectory}.stale-${randomUUID()}`;
  try {
    await rename(lockDirectory, retired);
  } catch (error: unknown) {
    if (errorCode(error) === "ENOENT") return;
    throw error;
  }
  await rm(retired, { force: true, recursive: true });
}

async function releaseOwnedLock(lockDirectory: string, token: string): Promise<void> {
  try {
    const owner = JSON.parse(
      await readFile(join(lockDirectory, "owner.json"), "utf8"),
    ) as Partial<LockOwner>;
    if (owner.token !== token) return;
  } catch {
    return;
  }
  const released = `${lockDirectory}.released-${token}`;
  try {
    await rename(lockDirectory, released);
  } catch (error: unknown) {
    if (errorCode(error) === "ENOENT") return;
    throw error;
  }
  await rm(released, { force: true, recursive: true });
}

async function releaseFailedCreation(lockDirectory: string, token: string): Promise<void> {
  try {
    const owner = JSON.parse(
      await readFile(join(lockDirectory, "owner.json"), "utf8"),
    ) as Partial<LockOwner>;
    if (typeof owner.token === "string" && owner.token !== token) return;
  } catch {
    // A failed first write leaves no valid competing owner inside the grace window.
  }
  await rm(lockDirectory, { force: true, recursive: true });
}

async function acquire(lockDirectory: string): Promise<string> {
  const deadline = Date.now() + LOCK_ACQUISITION_TIMEOUT_MS;
  const token = randomUUID();
  for (;;) {
    let created = false;
    try {
      await mkdir(lockDirectory);
      created = true;
      await writeFile(
        join(lockDirectory, "owner.json"),
        `${JSON.stringify({ createdAt: Date.now(), pid: process.pid, token } satisfies LockOwner)}\n`,
        "utf8",
      );
      return token;
    } catch (error: unknown) {
      if (created) {
        await releaseFailedCreation(lockDirectory, token);
        throw error;
      }
      if (errorCode(error) !== "EEXIST") throw error;
      await retireStaleLock(lockDirectory);
      if (Date.now() >= deadline) {
        throw Object.assign(new Error("production_build_lock_timeout"), { cause: error });
      }
      // suprnova-correctness-delay-allow: product-timer -- bounded lock retry cadence is the helper's tested production behavior
      await new Promise<void>((resolve) => {
        setTimeout(resolve, 25);
      });
    }
  }
}

export async function withProductionBuildLock<T>(
  task: () => T | Promise<T>,
  lockDirectory = LOCK_DIRECTORY,
): Promise<T> {
  const token = await acquire(lockDirectory);
  try {
    return await task();
  } finally {
    await releaseOwnedLock(lockDirectory, token);
  }
}
