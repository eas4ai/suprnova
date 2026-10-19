import { createHash, randomBytes } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { createServer } from 'node:http';
import type { AddressInfo } from 'node:net';
import { expect, test, type APIRequestContext, type Page, type Response } from '@playwright/test';

// AGENT-017 in a scaffolded Live kit, in Chromium under the kit's default
// security headers. `live_kit.rs` scaffolds the kit, builds and starts it
// with mail written to its log, and sets AGENTS_BASE_URL and
// LIVE_KIT_SERVER_LOG.
//
// AGENT-014: the notes page's two notes tools, called through WebMCP as a
// built-in agent calls them.

const serverLog = process.env.LIVE_KIT_SERVER_LOG ?? '';
const password = 'agents-kit-password-42';

test.skip(serverLog === '', 'Run through tests/agents_browser/live_kit.rs, which sets LIVE_KIT_SERVER_LOG.');

/** A loopback MCP client: the redirect URI it registers, on an origin other than the kit's, and every request that reached it. */
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

/** Register the client dynamically (RFC 7591); its id. */
async function registerClient(request: APIRequestContext, redirectUri: string): Promise<string> {
  const response = await request.post('/oauth/register', {
    data: {
      client_name: 'Browser Tool',
      redirect_uris: [redirectUri],
      token_endpoint_auth_method: 'none',
      grant_types: ['authorization_code'],
      response_types: ['code'],
    },
  });
  expect(response.status(), await response.text()).toBe(201);
  return (await response.json()).client_id as string;
}

/** The authorization request a client sends the browser to, and the PKCE verifier its code exchanges with. */
function authorize(clientId: string, redirectUri: string, baseURL: string): { path: string; verifier: string } {
  const verifier = randomBytes(32).toString('base64url');
  const query = new URLSearchParams({
    response_type: 'code',
    client_id: clientId,
    redirect_uri: redirectUri,
    code_challenge: createHash('sha256').update(verifier).digest('base64url'),
    code_challenge_method: 'S256',
    resource: `${baseURL}/mcp`,
    state: 'state-123',
  });
  return { path: `/oauth/authorize?${query}`, verifier };
}

let sequence = 0;

/** An address no other case uses. */
function freshEmail(label: string): string {
  sequence += 1;
  return `${label}-${Date.now()}-${sequence}@example.test`;
}

/** Wait until the Live runtime has taken over the form island `key`, so its submit runs the island's action. */
async function liveForm(page: Page, key: string): Promise<void> {
  await expect(page.locator(`[data-suprnova-live-document-key="${key}"][data-suprnova-live-status="connected"]`)).toHaveCount(1);
}

/** Fill and send the kit's Live registration form on the current page. */
async function registerThroughTheForm(page: Page, name: string, email: string): Promise<void> {
  await liveForm(page, 'register-form');
  await page.locator('#name').fill(name);
  await page.locator('#email').fill(email);
  await page.locator('#password').fill(password);
  await page.locator('#password_confirmation').fill(password);
  await page.getByRole('button', { name: 'Register' }).click();
}

/** The verification link the kit's `log` mail driver wrote for `email`, or '' while there is none. */
function verificationLinkIn(email: string): string {
  const lines = readFileSync(serverLog, 'utf8')
    .split('\n')
    .filter((line) => line.includes(email) && line.includes('/verify-email/verify?token='));
  const match = /\/verify-email\/verify\?token=([A-Za-z0-9_.~%-]+)/.exec(lines.at(-1) ?? '');
  return match ? `/verify-email/verify?token=${match[1]}` : '';
}

/** Register a new account through the kit's form and verify its address with the mailed link; the address. */
async function verifiedUser(page: Page, label: string): Promise<string> {
  const email = freshEmail(label);
  await page.goto('/register');
  await registerThroughTheForm(page, label, email);
  await page.waitForURL('**/verify-email');
  await expect.poll(() => verificationLinkIn(email)).not.toBe('');
  await page.goto(verificationLinkIn(email));
  await page.waitForURL('**/dashboard');
  return email;
}

/** An agent registers by `service_auth` for `email`: the claim page's address and the code it gives the user. */
async function startClaim(request: APIRequestContext, email: string): Promise<{ uri: string; code: string }> {
  const response = await request.post('/agent/identity', { data: { type: 'service_auth', login_hint: email } });
  expect(response.status(), await response.text()).toBe(200);
  const body = await response.json();
  return { uri: body.claim.verification_uri as string, code: body.claim.user_code as string };
}

/** A code that differs from `code` in its first digit. */
function wrongCode(code: string): string {
  return (code.startsWith('1') ? '2' : '1') + code.slice(1);
}

/** Every consent page refuses framing, caching and referrers, and keeps the kit's `form-action 'self'`. */
function expectConsentHeaders(response: Response | null): void {
  const headers = response?.headers() ?? {};
  expect(headers['x-frame-options']).toBe('DENY');
  expect(headers['cache-control']).toBe('no-store');
  expect(headers['referrer-policy']).toBe('no-referrer');
  expect(headers['content-security-policy']).toContain("frame-ancestors 'none'");
  expect(headers['content-security-policy']).toContain("form-action 'self'");
}

test('agent_017_live_kit_approval_reaches_the_client_under_the_default_security_headers', async ({ page, baseURL }) => {
  const client = await startClient();
  try {
    const clientId = await registerClient(page.request, client.redirectUri);
    // A guest the authorization endpoint sends to sign in registers
    // instead, and comes back to the approval page.
    await page.goto(authorize(clientId, client.redirectUri, baseURL!).path);
    await expect(page).toHaveURL(/\/login\?return_to=/);
    await page.getByRole('navigation', { name: 'Account' }).getByRole('link', { name: 'Register' }).click();
    await page.waitForURL('**/register');
    const approvalPage = page.waitForResponse((response) => response.url().includes('/oauth/approve?authorization='));
    await registerThroughTheForm(page, 'Approver', freshEmail('approver'));
    expectConsentHeaders(await approvalPage);
    await page.waitForURL(/\/oauth\/approve\?authorization=/);
    await expect(page.getByRole('heading', { level: 1 })).toHaveText('Allow Browser Tool to act for you?');
    await Promise.all([
      page.waitForURL((url) => url.href.startsWith(`${client.redirectUri}?`)),
      page.getByRole('button', { name: 'Allow' }).click(),
    ]);
    expect(client.hits).toHaveLength(1);
    const reached = client.hits[0];
    expect(reached.pathname).toBe('/callback');
    expect(reached.searchParams.get('code')).toBeTruthy();
    expect(reached.searchParams.get('state')).toBe('state-123');
    expect(reached.searchParams.get('iss')).toBe(baseURL);
  } finally {
    await client.close();
  }
});

test('agent_017_live_kit_claim_round_trip', async ({ page }) => {
  const email = await verifiedUser(page, 'claimer');
  const claim = await startClaim(page.request, email);
  const shown = await page.goto(claim.uri);
  expect(shown?.status()).toBe(200);
  expectConsentHeaders(shown);
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('Let an agent act for you?');
  await expect(page.getByText(`You are signed in as ${email}.`)).toBeVisible();
  await page.locator('#user_code').fill(claim.code);
  await Promise.all([page.waitForURL('**/connected-agents'), page.getByRole('button', { name: 'Allow' }).click()]);
  await expect(page.getByText('The agent is connected.')).toBeVisible();
  await expect(page.getByText('Registered with your email address')).toBeVisible();
});

test('agent_017_live_kit_wrong_claim_code_shows_the_attempts_left', async ({ page }) => {
  const email = await verifiedUser(page, 'mistyped');
  const claim = await startClaim(page.request, email);
  await page.goto(claim.uri);
  const allowed = Number(/(\d+) wrong entries allowed/.exec(await page.locator('main').innerText())?.[1]);
  expect(allowed).toBeGreaterThan(1);
  await page.locator('#user_code').fill(wrongCode(claim.code));
  const [answer] = await Promise.all([
    page.waitForResponse((response) => response.url().endsWith('/agent/claim') && response.request().method() === 'POST'),
    page.getByRole('button', { name: 'Allow' }).click(),
  ]);
  expect(answer.status()).toBe(422);
  expectConsentHeaders(answer);
  await expect(page.getByText(`That code is wrong. ${allowed - 1} attempts left.`)).toBeVisible();
  await expect(page.locator('#user_code')).toBeVisible();
});

test('agent_017_live_kit_connected_page_revokes', async ({ page, baseURL }) => {
  const client = await startClient();
  try {
    const email = await verifiedUser(page, 'revoker');
    const clientId = await registerClient(page.request, client.redirectUri);
    const request = authorize(clientId, client.redirectUri, baseURL!);
    await page.goto(request.path);
    await page.waitForURL(/\/oauth\/approve\?authorization=/);
    await Promise.all([
      page.waitForURL((url) => url.href.startsWith(`${client.redirectUri}?`)),
      page.getByRole('button', { name: 'Allow' }).click(),
    ]);
    // The client exchanges its code, which makes the grant it then holds.
    const tokens = await page.request.post('/oauth/token', {
      form: {
        grant_type: 'authorization_code',
        code: client.hits[0].searchParams.get('code') ?? '',
        redirect_uri: client.redirectUri,
        client_id: clientId,
        code_verifier: request.verifier,
        resource: `${baseURL}/mcp`,
      },
    });
    expect(tokens.status(), await tokens.text()).toBe(200);
    const claim = await startClaim(page.request, email);
    await page.goto(claim.uri);
    await page.locator('#user_code').fill(claim.code);
    await Promise.all([page.waitForURL('**/connected-agents'), page.getByRole('button', { name: 'Allow' }).click()]);

    await expect(page.getByRole('heading', { name: 'Browser Tool' })).toBeVisible();
    await page.getByRole('button', { name: 'Revoke Browser Tool' }).click();
    await expect(page.getByText('No apps can act for you.')).toBeVisible();
    await expect(page.getByText('The app can no longer act for you.')).toBeVisible();
    await page.getByRole('button', { name: 'Revoke this agent' }).click();
    await expect(page.getByText('No agents can act for you.')).toBeVisible();
    await expect(page.getByText('The agent can no longer act for you.')).toBeVisible();
  } finally {
    await client.close();
  }
});

// ---------------------------------------------------- the notes tools ----

/** Every tool the browser holds for the page, sorted. */
async function everyTool(page: Page): Promise<string[]> {
  return page.evaluate(async () => {
    const context = (document as unknown as { modelContext: { getTools(): Promise<{ name: string }[]> } })
      .modelContext;
    return (await context.getTools()).map((tool) => tool.name).sort();
  });
}

interface ToolResponse {
  readonly output?: {
    readonly structuredContent?: Record<string, unknown>;
    readonly isError?: boolean;
  };
}

/** An agent in the browser, through the DevTools `WebMCP` domain: it calls a tool the page registered and waits for the answer. */
async function browserAgent(page: Page) {
  const cdp = await page.context().newCDPSession(page);
  await cdp.send('WebMCP.enable' as never);
  const responses = new Map<string, ToolResponse>();
  let frameId = '';
  cdp.on('WebMCP.toolsAdded' as never, (event: { tools: { frameId: string }[] }) => {
    for (const tool of event.tools) frameId = tool.frameId;
  });
  cdp.on('WebMCP.toolResponded' as never, (event: ToolResponse & { invocationId: string }) => {
    responses.set(event.invocationId, event);
  });
  return async (toolName: string, input: Record<string, unknown>): Promise<ToolResponse> => {
    const { invocationId } = (await cdp.send('WebMCP.invokeTool' as never, {
      frameId,
      toolName,
      input,
    } as never)) as { invocationId: string };
    await expect.poll(() => responses.has(invocationId)).toBe(true);
    return responses.get(invocationId) as ToolResponse;
  };
}

test('agent_014_live_kit_notes_page_offers_the_notes_tools', async ({ page }) => {
  const invoke = await browserAgent(page);
  // A guest is sent to sign in, and no page a guest sees offers a tool.
  await page.goto('/notes');
  await expect(page).toHaveURL(/\/login$/);
  expect(await everyTool(page)).toEqual([]);

  await page.goto('/register');
  await registerThroughTheForm(page, 'Note Taker', freshEmail('notes'));
  await page.waitForURL('**/verify-email');
  await page.goto('/notes');
  await liveForm(page, 'notes-list');
  await expect.poll(() => everyTool(page)).toEqual(['notes.create', 'notes.search']);
  const kept = await page.evaluate(async () => {
    const context = (document as unknown as {
      modelContext: { getTools(): Promise<{ name: string; annotations: Record<string, boolean> }[]> };
    }).modelContext;
    return (await context.getTools()).find((tool) => tool.name === 'notes.search')?.annotations;
  });
  expect(kept).toMatchObject({ readOnlyHint: true, untrustedContentHint: true });

  // The agent writes a note, and the runtime renders the page's islands
  // again, so the list shows it with no document load.
  let loads = 0;
  page.on('load', () => {
    loads += 1;
  });
  const title = `Agent note ${Date.now()}`;
  const created = await invoke('notes.create', { title, body: 'Written through WebMCP.' });
  expect(created.output?.isError, JSON.stringify(created)).toBe(false);
  expect(created.output?.structuredContent).toMatchObject({ title });
  await expect(page.getByRole('link', { name: title })).toBeVisible();
  expect(loads).toBe(0);

  // The search reads its input from the query the page sends.
  const found = await invoke('notes.search', { query: 'WebMCP', limit: 5 });
  expect(found.output?.isError, JSON.stringify(found)).toBe(false);
  const notes = (found.output?.structuredContent?.notes ?? []) as { title: string }[];
  expect(notes.map((note) => note.title)).toEqual([title]);
});
