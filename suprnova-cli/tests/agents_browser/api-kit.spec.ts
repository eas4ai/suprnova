import { createHash, randomBytes } from 'node:crypto';
import { createServer } from 'node:http';
import type { AddressInfo } from 'node:net';
import { expect, test, type Page } from '@playwright/test';

// AGENT-017 for the API kit in Chromium, under the security headers the
// kit installs by default: the framework's default approval and claim
// pages send a guest to the kit's `/login`, and once signed in there the
// guest's decision reaches the client. Run through
// `tests/agents_browser/api_kit.rs`, which scaffolds and starts the kit and
// gives it a verified account.

const email = process.env.API_KIT_EMAIL ?? '';
const password = process.env.API_KIT_PASSWORD ?? '';

test.skip(email === '', 'Run through tests/agents_browser/api_kit.rs, which sets API_KIT_EMAIL.');

/** A loopback MCP client: the redirect URI it registers, on an origin other than the application's, and every request that reached it. */
interface Client {
  redirectUri: string;
  hits: URL[];
  close: () => Promise<void>;
}

async function startClient(): Promise<Client> {
  const hits: URL[] = [];
  const server = createServer((request, response) => {
    hits.push(new URL(request.url ?? '/', 'http://127.0.0.1'));
    response.writeHead(200, { 'content-type': 'text/plain' });
    response.end('the client got the answer');
  });
  await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve));
  const { port } = server.address() as AddressInfo;
  return {
    redirectUri: `http://127.0.0.1:${port}/callback`,
    hits,
    close: () => new Promise<void>((resolve) => server.close(() => resolve())),
  };
}

/** Sign in on the kit's sign-in page, which the browser is on, and wait to land on `destination`. */
async function signIn(page: Page, destination: RegExp) {
  expect(new URL(page.url()).pathname).toBe('/login');
  await page.getByLabel('Email').fill(email);
  await page.getByLabel('Password').fill(password);
  await Promise.all([page.waitForURL(destination), page.getByRole('button', { name: 'Sign in' }).click()]);
}

test('agent_017_api_kit_approval_reaches_the_client_through_the_kit_sign_in', async ({ page, request, baseURL }) => {
  const client = await startClient();
  try {
    const registered = await request.post('/oauth/register', {
      data: {
        client_name: 'Browser Tool',
        redirect_uris: [client.redirectUri],
        token_endpoint_auth_method: 'none',
        grant_types: ['authorization_code'],
        response_types: ['code'],
      },
    });
    expect(registered.status(), await registered.text()).toBe(201);
    const clientId = (await registered.json()).client_id as string;
    const verifier = randomBytes(32).toString('base64url');
    const challenge = createHash('sha256').update(verifier).digest('base64url');
    const resource = `${baseURL}/mcp`;
    const query = new URLSearchParams({
      response_type: 'code',
      client_id: clientId,
      redirect_uri: client.redirectUri,
      code_challenge: challenge,
      code_challenge_method: 'S256',
      resource,
      state: 'state-123',
    });

    // A guest: the authorization endpoint sends the browser to the kit's
    // sign-in, which refuses framing under the default policy.
    const signInPage = await page.goto(`/oauth/authorize?${query}`);
    expect(signInPage?.status()).toBe(200);
    expect(signInPage?.headers()['x-frame-options']).toBe('DENY');
    expect(signInPage?.headers()['content-security-policy']).toContain("frame-ancestors 'none'");
    expect(signInPage?.headers()['content-security-policy']).toContain("form-action 'self'");
    await signIn(page, /\/oauth\/approve\?authorization=/);

    // The framework's default approval page, then the client.
    await expect(page.getByRole('heading', { name: 'Allow Browser Tool to act for you?' })).toBeVisible();
    await Promise.all([
      page.waitForURL((url) => url.href.startsWith(`${client.redirectUri}?`)),
      page.getByRole('button', { name: 'Allow' }).click(),
    ]);
    expect(client.hits).toHaveLength(1);
    const reached = client.hits[0];
    expect(reached.pathname).toBe('/callback');
    expect(reached.searchParams.get('state')).toBe('state-123');
    expect(reached.searchParams.get('iss')).toBe(baseURL);
    const code = reached.searchParams.get('code');
    expect(code).toBeTruthy();

    // The code is good: the client exchanges it for a token.
    const token = await request.post('/oauth/token', {
      form: {
        grant_type: 'authorization_code',
        code: code!,
        redirect_uri: client.redirectUri,
        client_id: clientId,
        code_verifier: verifier,
        resource,
      },
    });
    expect(token.status(), await token.text()).toBe(200);
    expect((await token.json()).access_token).toBeTruthy();
  } finally {
    await client.close();
  }
});

test('agent_017_api_kit_claim_reaches_the_agent_through_the_kit_sign_in', async ({ page, request, baseURL }) => {
  // The agent registers under `service_auth`, naming the account's
  // address, and hands the person its link and code.
  const registered = await request.post('/agent/identity', {
    data: { type: 'service_auth', login_hint: email },
  });
  expect(registered.status(), await registered.text()).toBe(200);
  const registration = await registered.json();

  // A guest: the claim page sends the browser to the kit's sign-in.
  await page.goto(registration.claim.verification_uri as string);
  await signIn(page, /\/agent\/claim\?claim_attempt_token=/);

  // The framework's default claim page takes the code.
  await expect(page.getByRole('heading', { name: 'Let an agent act for you?' })).toBeVisible();
  await page.getByLabel('Code').fill(registration.claim.user_code as string);
  await page.getByRole('button', { name: 'Allow' }).click();
  await expect(page.getByRole('heading', { name: 'The agent is connected' })).toBeVisible();

  // The claim reached the agent: its claim token now buys an access token.
  const token = await request.post('/oauth/token', {
    form: {
      grant_type: 'urn:workos:agent-auth:grant-type:claim',
      claim_token: registration.claim_token as string,
      resource: `${baseURL}/mcp`,
    },
  });
  expect(token.status(), await token.text()).toBe(200);
  expect((await token.json()).access_token).toBeTruthy();
});
