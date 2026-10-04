import { expect, type APIRequestContext } from "@playwright/test";

const REFERENCE_ORIGIN = "http://127.0.0.1:4175";

/**
 * Runs the reference host's reset between tests (LIVE-039).
 *
 * A test that ends while one of its upload requests is still pending closes
 * the connection, and the host restores that upload with its active lease so
 * a retry in the same test could still finish it. The host's clock is fixed,
 * so nothing else ever ends the upload, and the strict reset each test runs
 * on its own uploads would refuse for every later test. This reset releases
 * the pause and disarms the faults the finished test left, cancels the
 * uploads it left unfinished, and then resets the creation window, so one
 * stalled request fails only its own test.
 *
 * Polled, not posted once: the host answers 409 while an operation still
 * holds an upload, which ends on its own.
 */
export async function resetReferenceUploadsBetweenTests(
  request: APIRequestContext,
  origin = REFERENCE_ORIGIN,
): Promise<void> {
  await expect
    .poll(async () => {
      const reset = await request.post(
        `${origin}/__test/iteration-004/control/upload/reset-between-tests`,
      );
      return reset.status();
    })
    .toBe(204);
}
