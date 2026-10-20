import { createHash, randomBytes } from 'node:crypto';
import { createServer } from 'node:http';
import type { AddressInfo } from 'node:net';
import { expect, test, type APIRequestContext, type Page } from '@playwright/test';

// AGENT-014 and AGENT-015 in a scaffolded Inertia kit, in Chromium with
// WebMCP enabled, observed as a built-in agent observes them: through the
// DevTools `WebMCP` domain and `document.modelContext`, each call made by
// `WebMCP.invokeTool`. `kits.rs` scaffolds the kit, adds the fixture pages
// and routes of `kit_fixtures/app`, starts the application and sets
// KIT_NAME and KIT_BASE_URL.
//
// AGENT-017: the kit's own approval and claim pages, under the security
// headers the scaffold's `.env` names.

const kit = process.env.KIT_NAME ?? '';
const password = 'agents-kit-password-42';

test.skip(kit === '', 'Run through tests/agents_browser/kits.rs, which sets KIT_NAME.');

interface ToolResponse {
  readonly status: string;
  readonly output?: {
    readonly structuredContent?: Record<string, unknown>;
    readonly isError?: boolean;
    readonly content?: { text: string }[];
  };
}

interface Recorded {
  readonly method: string;
  readonly path: string;
  readonly tool: string | null;
  readonly inertia: boolean;
  readonly csrf: boolean;
  readonly cookie: boolean;
}

interface Descriptor {
  readonly name: string;
  readonly title?: string;
  readonly description: string;
  readonly inputSchema: Record<string, unknown>;
  readonly annotations: Record<string, boolean>;
}

// Every descriptor the page hands `registerTool`, recorded before the real
// call, so a test sees `consequentialHint`, which Chromium does not keep.
test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    const recorded: unknown[] = [];
    Object.defineProperty(window, '__registered', { value: recorded });
    const context = (document as unknown as { modelContext?: Record<string, unknown> }).modelContext;
    if (context === undefined || typeof context.registerTool !== 'function') return;
    const register = (context.registerTool as (...args: unknown[]) => unknown).bind(context);
    context.registerTool = (tool: Record<string, unknown>, options: unknown) => {
      recorded.push(
        JSON.parse(
          JSON.stringify({
            name: tool.name,
            title: tool.title,
            description: tool.description,
            inputSchema: tool.inputSchema,
            annotations: tool.annotations,
          }),
        ),
      );
      return register(tool, options);
    };
  });
});

async function webmcp(page: Page) {
  const cdp = await page.context().newCDPSession(page);
  await cdp.send('WebMCP.enable' as never);
  const added: string[] = [];
  const removed: string[] = [];
  const responses = new Map<string, ToolResponse>();
  let frameId = '';
  cdp.on('WebMCP.toolsAdded' as never, (event: { tools: { name: string; frameId: string }[] }) => {
    for (const tool of event.tools) {
      added.push(tool.name);
      frameId = tool.frameId;
    }
  });
  cdp.on('WebMCP.toolsRemoved' as never, (event: { tools: { name: string }[] }) => {
    for (const tool of event.tools) removed.push(tool.name);
  });
  cdp.on('WebMCP.toolResponded' as never, (event: ToolResponse & { invocationId: string }) => {
    responses.set(event.invocationId, event);
  });
  return {
    added,
    removed,
    async invoke(toolName: string, input: Record<string, unknown>): Promise<ToolResponse> {
      const { invocationId } = (await cdp.send('WebMCP.invokeTool' as never, {
        frameId,
        toolName,
        input,
      } as never)) as { invocationId: string };
      await expect.poll(() => responses.has(invocationId)).toBe(true);
      return responses.get(invocationId) as ToolResponse;
    },
  };
}

/** The fixture's tools the browser holds for the page, sorted. */
async function registered(page: Page): Promise<string[]> {
  return page.evaluate(async () => {
    const context = (document as unknown as { modelContext: { getTools(): Promise<{ name: string }[]> } })
      .modelContext;
    return (await context.getTools())
      .map((tool) => tool.name)
      .filter((name) => name.startsWith('fixture.'))
      .sort();
  });
}

async function descriptors(page: Page): Promise<Descriptor[]> {
  return page.evaluate(() => (window as unknown as { __registered: Descriptor[] }).__registered);
}

async function requests(page: Page): Promise<Recorded[]> {
  const body = (await (await page.request.get('/agent-fixture/requests')).json()) as { requests: Recorded[] };
  return body.requests;
}

/** Count the document loads of `page` from now on: an Inertia visit makes none. */
function documentLoads(page: Page): () => number {
  let loads = 0;
  page.on('load', () => {
    loads += 1;
  });
  return () => loads;
}

async function register(page: Page) {
  const stamp = `${Date.now()}${Math.floor(Math.random() * 1000)}`;
  await page.goto('/register');
  await page.locator('#name').fill(`Agent ${stamp}`);
  await page.locator('#email').fill(`agent-${stamp}@example.test`);
  await page.locator('#password').fill(password);
  await page.locator('#password_confirmation').fill(password);
  await page.getByRole('button', { name: 'Register', exact: true }).click();
  await expect(page).toHaveURL(/\/(verify-email|dashboard)$/);
}

const FIRST_PAGE_TOOLS = ['fixture.bump', 'fixture.help', 'fixture.peek', 'fixture.widget.ping'];

test(`agent_014_${kit}_page_registers_the_server_tools_the_viewer_may_call`, async ({ page }) => {
  await page.goto('/agent-fixture/first');
  // A guest is never offered the tool only a signed-in user may call.
  await expect.poll(() => registered(page)).toEqual(FIRST_PAGE_TOOLS);
  await register(page);
  await page.goto('/agent-fixture/first');
  await expect
    .poll(() => registered(page))
    .toEqual(['fixture.bump', 'fixture.help', 'fixture.mine', 'fixture.peek', 'fixture.widget.ping']);
});

test(`agent_014_${kit}_server_tool_annotations_follow_the_markings`, async ({ page }) => {
  await page.goto('/agent-fixture/first');
  await expect.poll(() => registered(page)).toEqual(FIRST_PAGE_TOOLS);
  const sent = await descriptors(page);
  const sentFor = (name: string) => sent.find((tool) => tool.name === name);
  expect(sentFor('fixture.peek')?.annotations).toEqual({
    readOnlyHint: true,
    untrustedContentHint: true,
    consequentialHint: false,
    debugging: false,
  });
  expect(sentFor('fixture.bump')?.annotations).toEqual({
    readOnlyHint: false,
    untrustedContentHint: false,
    consequentialHint: false,
    debugging: false,
  });
  expect(sentFor('fixture.bump')?.title).toBe('Bump');
  expect(sentFor('fixture.bump')?.description).toBe('Add to the counter the first page shows.');
  // What Chromium kept: the two hints it knows.
  const kept = await page.evaluate(async () => {
    const context = (document as unknown as {
      modelContext: { getTools(): Promise<{ name: string; annotations: Record<string, boolean> }[]> };
    }).modelContext;
    return (await context.getTools()).find((tool) => tool.name === 'fixture.peek')?.annotations;
  });
  expect(kept).toMatchObject({ readOnlyHint: true, untrustedContentHint: true });

  await page.getByTestId('to-second').click();
  await expect.poll(() => registered(page)).toEqual(['fixture.help', 'fixture.reset']);
  const reset = (await descriptors(page)).find((tool) => tool.name === 'fixture.reset');
  expect(reset?.annotations).toEqual({
    readOnlyHint: false,
    untrustedContentHint: false,
    consequentialHint: true,
    debugging: false,
  });
});

test(`agent_014_${kit}_inertia_visit_replaces_the_page_tools`, async ({ page }) => {
  const agent = await webmcp(page);
  await page.goto('/agent-fixture/first');
  await expect.poll(() => registered(page)).toEqual(FIRST_PAGE_TOOLS);
  const loads = documentLoads(page);
  await page.getByTestId('to-second').click();
  await expect(page.getByRole('heading', { name: 'Second' })).toBeVisible();
  // The route's tools leave, the group's shared tool stays registered.
  await expect.poll(() => registered(page)).toEqual(['fixture.help', 'fixture.reset']);
  expect(loads()).toBe(0);
  expect(agent.removed).toEqual(expect.arrayContaining(['fixture.bump', 'fixture.peek']));
  expect(agent.removed).not.toContain('fixture.help');
});

test(`agent_014_${kit}_history_back_restores_that_pages_tools`, async ({ page }) => {
  await page.goto('/agent-fixture/first');
  await expect.poll(() => registered(page)).toEqual(FIRST_PAGE_TOOLS);
  await page.getByTestId('to-second').click();
  await expect.poll(() => registered(page)).toEqual(['fixture.help', 'fixture.reset']);
  await page.goBack();
  await expect(page.getByRole('heading', { name: 'First' })).toBeVisible();
  await expect.poll(() => registered(page)).toEqual(FIRST_PAGE_TOOLS);
});

test(`agent_014_${kit}_server_tool_call_carries_session_csrf_and_header`, async ({ page }) => {
  const agent = await webmcp(page);
  await register(page);
  await page.goto('/agent-fixture/first');
  await expect.poll(() => registered(page)).toContain('fixture.mine');
  const before = (await requests(page)).length;
  const response = await agent.invoke('fixture.mine', {});
  expect(response.output?.isError).toBe(false);
  // The route's own guard saw the session's user, through a browser call.
  expect(response.output?.structuredContent).toMatchObject({ browser: true });
  expect(response.output?.structuredContent?.user).toEqual(expect.any(String));
  const call = (await requests(page))
    .slice(before)
    .find((request) => request.path === '/agent-fixture/mine');
  expect(call).toMatchObject({ method: 'POST', tool: 'fixture.mine', csrf: true, cookie: true });
});

test(`agent_014_${kit}_write_tool_reloads_the_page_props`, async ({ page }) => {
  const agent = await webmcp(page);
  await page.goto('/agent-fixture/first');
  await expect.poll(() => registered(page)).toEqual(FIRST_PAGE_TOOLS);
  const shown = Number(await page.getByTestId('counter').textContent());
  const loads = documentLoads(page);
  const before = (await requests(page)).length;
  const response = await agent.invoke('fixture.bump', { by: 2 });
  expect(response.output?.isError).toBe(false);
  expect(response.output?.structuredContent).toMatchObject({ counter: shown + 2, kind: 'public' });
  // The screen shows what the agent did, through an Inertia reload.
  await expect(page.getByTestId('counter')).toHaveText(String(shown + 2));
  expect(loads()).toBe(0);
  const after = (await requests(page)).slice(before);
  const call = after.findIndex((request) => request.tool === 'fixture.bump');
  const reload = after.findIndex(
    (request) => request.method === 'GET' && request.path === '/agent-fixture/first' && request.inertia,
  );
  expect(call).toBeGreaterThanOrEqual(0);
  expect(reload).toBeGreaterThan(call);
  // A read-only tool reloads nothing.
  const quiet = (await requests(page)).length;
  expect((await agent.invoke('fixture.peek', {})).output?.isError).toBe(false);
  expect(
    (await requests(page)).slice(quiet).filter((request) => request.path === '/agent-fixture/first'),
  ).toEqual([]);
});

test(`agent_015_${kit}_component_tool_registers_and_leaves_with_its_component`, async ({ page }) => {
  const agent = await webmcp(page);
  await page.goto('/agent-fixture/first');
  await expect.poll(() => registered(page)).toContain('fixture.widget.ping');
  expect((await agent.invoke('fixture.widget.ping', {})).output?.structuredContent).toEqual({ pong: true });
  await page.getByTestId('toggle').click();
  await expect(page.getByTestId('widget')).toHaveCount(0);
  await expect.poll(() => registered(page)).not.toContain('fixture.widget.ping');
  await expect.poll(() => agent.removed).toContain('fixture.widget.ping');
  await page.getByTestId('toggle').click();
  await expect.poll(() => registered(page)).toContain('fixture.widget.ping');
});

test(`agent_015_${kit}_form_tool_registers_the_generated_schema`, async ({ page }) => {
  await page.goto('/agent-fixture/form');
  await expect
    .poll(() => registered(page))
    .toEqual(['fixture.help', 'fixture.notes.create', 'fixture.notes.create_and_leave', 'fixture.notes.purge']);
  const server = await (await page.request.get('/agent-fixture/schema')).json();
  const sent = (await descriptors(page)).find((tool) => tool.name === 'fixture.notes.create');
  // The schema `generate-types` wrote is the one the server derives.
  expect(sent?.inputSchema).toEqual(server);
  expect(sent?.annotations).toEqual({
    readOnlyHint: false,
    untrustedContentHint: false,
    consequentialHint: false,
    debugging: false,
  });
});

test(`agent_015_${kit}_form_tool_fills_and_submits_through_inertia`, async ({ page }) => {
  const agent = await webmcp(page);
  await page.goto('/agent-fixture/form');
  await expect.poll(() => registered(page)).toContain('fixture.notes.create');
  const loads = documentLoads(page);
  const before = (await requests(page)).length;
  const title = `Agent note ${Date.now()}`;
  const response = await agent.invoke('fixture.notes.create', { title });
  expect(response.output?.isError).toBe(false);
  expect(response.output?.structuredContent).toMatchObject({ outcome: 'submitted', component: 'AgentFixture/Form' });
  await expect(page.getByTestId('notes')).toContainText(title);
  expect(loads()).toBe(0);
  // An ordinary Inertia visit: no tool-call header, the route's own checks.
  const submit = (await requests(page))
    .slice(before)
    .find((request) => request.method === 'POST' && request.path === '/agent-fixture/notes');
  expect(submit).toMatchObject({ inertia: true, tool: null, csrf: true });
});

test(`agent_015_${kit}_form_tool_returns_each_invalid_field`, async ({ page }) => {
  const agent = await webmcp(page);
  await page.goto('/agent-fixture/form');
  await expect.poll(() => registered(page)).toContain('fixture.notes.create');
  const response = await agent.invoke('fixture.notes.create', { title: 'ab' });
  expect(response.status).not.toBe('Error');
  expect(response.output?.isError).toBe(true);
  expect(response.output).not.toHaveProperty('content');
  expect(response.output?.structuredContent).toMatchObject({
    errors: { title: 'Give the note a title of 3 to 80 characters.' },
  });
  expect(response.output?.structuredContent).toMatchObject({ outcome: 'invalid' });
  // The user sees the same form the agent filled, with the same error.
  await expect(page.getByTestId('title')).toHaveValue('ab');
  await expect(page.getByTestId('title-error')).toHaveText('Give the note a title of 3 to 80 characters.');
});

test(`agent_015_${kit}_destructive_form_tool_leaves_the_submit_to_the_user`, async ({ page }) => {
  const agent = await webmcp(page);
  await page.goto('/agent-fixture/form');
  await expect.poll(() => registered(page)).toContain('fixture.notes.purge');
  const sent = (await descriptors(page)).find((tool) => tool.name === 'fixture.notes.purge');
  expect(sent?.annotations.consequentialHint).toBe(true);
  const before = (await requests(page)).length;
  const response = await agent.invoke('fixture.notes.purge', { confirm: 'yes' });
  expect(response.output?.structuredContent).toEqual({ outcome: 'filled', submitted: false });
  await expect(page.getByTestId('confirm')).toHaveValue('yes');
  const purges = async () =>
    (await requests(page))
      .slice(before)
      .filter((request) => request.method === 'POST' && request.path === '/agent-fixture/purge');
  expect(await purges()).toEqual([]);
  // Only the user's own click submits it.
  await page.getByTestId('purge').click();
  await expect.poll(async () => (await purges()).length).toBe(1);
  expect((await purges())[0]).toMatchObject({ inertia: true, tool: null });
});

test(`agent_015_${kit}_form_tool_answers_when_its_submit_leaves_the_page`, async ({ page }) => {
  const agent = await webmcp(page);
  // The submit redirects to the first page, so the form and its tool leave
  // the page while the agent's call still waits for its answer. Five times,
  // so a result the browser drops at the tool's removal shows.
  for (let round = 0; round < 5; round += 1) {
    await page.goto('/agent-fixture/form');
    await expect.poll(() => registered(page)).toContain('fixture.notes.create_and_leave');
    const title = `Leaving note ${round} ${Date.now()}`;
    const response = await agent.invoke('fixture.notes.create_and_leave', { title });
    expect(response.output?.isError, `round ${round}: ${JSON.stringify(response)}`).toBe(false);
    expect(response.output?.structuredContent).toMatchObject({
      outcome: 'submitted',
      component: 'AgentFixture/First',
    });
    await expect(page.getByRole('heading', { name: 'First' })).toBeVisible();
    await expect.poll(() => registered(page)).not.toContain('fixture.notes.create_and_leave');
  }
});

// ------------------------------------------------------------ AGENT-017 ----

/** Register a new account through the kit's form; its address. The browser is signed in afterwards. */
async function signUp(page: Page): Promise<string> {
  const stamp = `${Date.now()}${Math.floor(Math.random() * 1000)}`;
  const email = `consent-${stamp}@example.test`;
  await page.goto('/register');
  await page.locator('#name').fill(`Consent ${stamp}`);
  await page.locator('#email').fill(email);
  await page.locator('#password').fill(password);
  await page.locator('#password_confirmation').fill(password);
  await page.getByRole('button', { name: 'Register', exact: true }).click();
  await expect(page).toHaveURL(/\/(verify-email|dashboard)$/);
  return email;
}

/** Every tool the browser holds for the page, sorted: a consent page offers none. */
async function everyTool(page: Page): Promise<string[]> {
  return page.evaluate(async () => {
    const context = (document as unknown as { modelContext: { getTools(): Promise<{ name: string }[]> } })
      .modelContext;
    return (await context.getTools()).map((tool) => tool.name).sort();
  });
}

/** Record the signed-in user's address as verified, as the mailed link would. */
async function verifyMe(page: Page) {
  const xsrf = (await page.context().cookies()).find((cookie) => cookie.name === 'XSRF-TOKEN')?.value ?? '';
  const verified = await page.request.post('/agent-fixture/verify-me', {
    headers: { 'X-XSRF-TOKEN': decodeURIComponent(xsrf) },
  });
  expect(verified.status(), await verified.text()).toBe(200);
}

/** A loopback MCP client: the redirect URI it registers, on another origin than the application's, and every request that reached it. */
interface LoopbackClient {
  redirectUri: string;
  hits: URL[];
  close: () => Promise<void>;
}

async function startLoopbackClient(): Promise<LoopbackClient> {
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

/** Register the client dynamically (RFC 7591), with no cookie, as a client does; its id. */
async function registerClient(request: APIRequestContext, redirectUri: string): Promise<string> {
  const response = await request.post('/oauth/register', {
    data: {
      client_name: 'Kit Browser Tool',
      redirect_uris: [redirectUri],
      token_endpoint_auth_method: 'none',
      grant_types: ['authorization_code'],
      response_types: ['code'],
    },
  });
  expect(response.status(), await response.text()).toBe(201);
  return (await response.json()).client_id as string;
}

/** Register an agent by `service_auth` for `email`, with no cookie, as an agent does. */
async function serviceAuth(request: APIRequestContext, email: string) {
  const response = await request.post('/agent/identity', { data: { type: 'service_auth', login_hint: email } });
  expect(response.status(), await response.text()).toBe(200);
  return (await response.json()) as {
    claim_token: string;
    claim: { verification_uri: string; user_code: string };
  };
}

test(`agent_017_${kit}_approval_page_round_trip`, async ({ page, request, baseURL }) => {
  const client = await startLoopbackClient();
  try {
    const clientId = await registerClient(request, client.redirectUri);
    const email = await signUp(page);
    // A guest from here on: the authorization request sends them to sign in.
    await page.context().clearCookies();
    const verifier = randomBytes(32).toString('base64url');
    const query = new URLSearchParams({
      response_type: 'code',
      client_id: clientId,
      redirect_uri: client.redirectUri,
      code_challenge: createHash('sha256').update(verifier).digest('base64url'),
      code_challenge_method: 'S256',
      resource: `${baseURL}/mcp`,
      state: 'state-123',
    });
    await page.goto(`/oauth/authorize?${query}`);
    await expect(page).toHaveURL(/\/login\?return_to=/);
    await page.locator('#email').fill(email);
    await page.locator('#password').fill(password);
    // The sign-in continues to the authorization request by a location
    // visit, which sends the browser to the kit's approval page.
    const approval = page.waitForResponse((response) => response.url().includes('/oauth/approve?authorization='));
    await page.getByRole('button', { name: 'Sign in', exact: true }).click();
    const document = await approval;
    expect(document.status()).toBe(200);
    // The default preset's policy, with framing refused: its `form-action
    // 'self'` stays, and the decision still reaches the client.
    const csp = document.headers()['content-security-policy'] ?? '';
    expect(csp).toContain("form-action 'self'");
    expect(csp).toContain("frame-ancestors 'none'");
    expect(document.headers()['x-frame-options']).toBe('DENY');
    expect(document.headers()['referrer-policy']).toBe('no-referrer');
    expect(document.headers()['cache-control']).toContain('no-store');
    await expect(page.getByRole('heading', { name: 'Allow Kit Browser Tool to act for you?' })).toBeVisible();
    await expect(page.getByText(client.redirectUri)).toBeVisible();
    expect(await everyTool(page)).toEqual([]);

    await Promise.all([
      page.waitForURL((url) => url.href.startsWith(`${client.redirectUri}?`)),
      page.getByRole('button', { name: 'Allow', exact: true }).click(),
    ]);
    expect(client.hits).toHaveLength(1);
    const reached = client.hits[0];
    expect(reached.pathname).toBe('/callback');
    expect(reached.searchParams.get('code')).toBeTruthy();
    expect(reached.searchParams.get('state')).toBe('state-123');
    expect(reached.searchParams.get('iss')).toBe(baseURL);

    // The client exchanges the code, and its grant is live.
    const tokens = await request.post('/oauth/token', {
      form: {
        grant_type: 'authorization_code',
        code: reached.searchParams.get('code') ?? '',
        redirect_uri: client.redirectUri,
        client_id: clientId,
        code_verifier: verifier,
        resource: `${baseURL}/mcp`,
      },
    });
    expect(tokens.status(), await tokens.text()).toBe(200);

    // The connected-agents page lists the client and revokes it.
    await page.goto('/connected-agents');
    await expect(page.getByText('Kit Browser Tool')).toBeVisible();
    await page.getByRole('button', { name: 'Revoke', exact: true }).click();
    await expect(page.getByText('No apps can act for you.')).toBeVisible();
    await expect(page.getByText('Kit Browser Tool')).toHaveCount(0);
  } finally {
    await client.close();
  }
});

test(`agent_017_${kit}_claim_page_round_trip`, async ({ page, request, baseURL }) => {
  const email = await signUp(page);
  await verifyMe(page);
  const registration = await serviceAuth(request, email);
  await page.goto(registration.claim.verification_uri);
  await expect(page.getByRole('heading', { name: 'Let an agent act for you?' })).toBeVisible();
  await expect(page.getByText('An agent that named your email address asks to act as you.')).toBeVisible();
  expect(await everyTool(page)).toEqual([]);
  await page.locator('#user_code').fill(registration.claim.user_code);
  await page.getByRole('button', { name: 'Allow', exact: true }).click();
  await expect(page).toHaveURL(/\/connected-agents$/);
  await expect(page.getByText('The agent is connected.')).toBeVisible();
  await expect(page.getByText('Registered with your email address')).toBeVisible();

  // The agent's poll gets its tokens.
  const tokens = await request.post('/oauth/token', {
    form: {
      grant_type: 'urn:workos:agent-auth:grant-type:claim',
      claim_token: registration.claim_token,
      resource: `${baseURL}/mcp`,
    },
  });
  expect(tokens.status(), await tokens.text()).toBe(200);
  expect((await tokens.json()).access_token).toBeTruthy();
});

test(`agent_017_${kit}_wrong_claim_code_shows_the_attempts_left`, async ({ page, request }) => {
  const email = await signUp(page);
  await verifyMe(page);
  const registration = await serviceAuth(request, email);
  await page.goto(registration.claim.verification_uri);
  const allowed = (await page.getByText(/wrong entries allowed/).textContent()) ?? '';
  const attempts = Number(/(\d+) wrong entries allowed/.exec(allowed)?.[1]);
  expect(attempts).toBeGreaterThan(1);
  const wrong = registration.claim.user_code === '000000' ? '111111' : '000000';
  await page.locator('#user_code').fill(wrong);
  await page.getByRole('button', { name: 'Allow', exact: true }).click();
  await expect(page.getByText(`That code is wrong. ${attempts - 1} attempts left.`)).toBeVisible();
  await expect(page).toHaveURL(/\/agent\/claim\?claim_attempt_token=/);
  await expect(page.getByText(`${attempts - 1} wrong entries allowed`)).toBeVisible();
});

test(`agent_017_${kit}_form_tool_cannot_approve`, async ({ page, request, baseURL }) => {
  const client = await startLoopbackClient();
  try {
    const clientId = await registerClient(request, client.redirectUri);
    await signUp(page);
    const query = new URLSearchParams({
      response_type: 'code',
      client_id: clientId,
      redirect_uri: client.redirectUri,
      code_challenge: createHash('sha256').update(randomBytes(32).toString('base64url')).digest('base64url'),
      code_challenge_method: 'S256',
      resource: `${baseURL}/mcp`,
      state: 'state-123',
    });
    await page.goto(`/oauth/authorize?${query}`);
    await expect(page).toHaveURL(/\/oauth\/approve\?authorization=/);
    const approval = new URL(page.url());
    const id = approval.searchParams.get('authorization') ?? '';

    // An application's page with a form tool that posts the approval form,
    // with the user's own session and CSRF token.
    const agent = await webmcp(page);
    await page.goto('/agent-fixture/consent');
    await expect.poll(() => registered(page)).toContain('fixture.consent.approve');
    const posted = page.waitForResponse(
      (response) => response.request().method() === 'POST' && new URL(response.url()).pathname === '/oauth/approve',
    );
    await agent.invoke('fixture.consent.approve', { authorization: id, decision: 'approve' });
    const refused = await posted;
    expect(refused.request().headers()['suprnova-form-tool']).toBe('1');
    expect(refused.status()).toBe(403);

    // The authorization is still the user's to decide, and the client got
    // nothing.
    expect(client.hits).toHaveLength(0);
    const pending = await page.request.get(`${approval.pathname}${approval.search}`, { headers: { Accept: 'text/html' } });
    expect(pending.status(), await pending.text()).toBe(200);
  } finally {
    await client.close();
  }
});

test(`agent_017_${kit}_notes_after_a_claim_keep_their_form_errors`, async ({ page, request }) => {
  const email = await signUp(page);
  await verifyMe(page);
  const registration = await serviceAuth(request, email);
  await page.goto(registration.claim.verification_uri);
  await page.locator('#user_code').fill(registration.claim.user_code);
  await page.getByRole('button', { name: 'Allow', exact: true }).click();
  await expect(page).toHaveURL(/\/connected-agents$/);

  // The document came from the claim page, which sends no referrer, so the
  // notes form posts with none: a failed rule must still return to it, not
  // to the last script the page loaded.
  await page.getByRole('link', { name: 'Notes', exact: true }).click();
  await expect(page).toHaveURL(/\/notes$/);
  await page.locator('#title').fill('   ');
  const posted = page.waitForResponse(
    (response) => response.request().method() === 'POST' && new URL(response.url()).pathname === '/notes',
  );
  await page.getByRole('button', { name: 'Save note' }).click();
  const refused = await posted;
  expect(refused.status()).toBe(303);
  expect(new URL(refused.headers()['location'] ?? '', 'http://kit.test').pathname).toBe('/notes');
  await expect(page.getByText('Give the note a title of 1 to 255 characters.')).toBeVisible();
  await expect(page).toHaveURL(/\/notes$/);
});

// ------------------------------------------------- the kit's notes tools ----

test(`agent_014_${kit}_notes_page_offers_the_notes_tools`, async ({ page }) => {
  const agent = await webmcp(page);
  // A guest is sent to sign in, and no page a guest sees offers a tool.
  await page.goto('/notes');
  await expect(page).toHaveURL(/\/login$/);
  expect(await everyTool(page)).toEqual([]);

  await register(page);
  await page.goto('/notes');
  await expect.poll(() => everyTool(page)).toEqual(['notes.create', 'notes.search']);
  const sent = await descriptors(page);
  expect(sent.find((tool) => tool.name === 'notes.search')?.annotations).toEqual({
    readOnlyHint: true,
    untrustedContentHint: true,
    consequentialHint: false,
    debugging: false,
  });
  expect(sent.find((tool) => tool.name === 'notes.create')?.annotations).toEqual({
    readOnlyHint: false,
    untrustedContentHint: false,
    consequentialHint: false,
    debugging: false,
  });

  // The agent writes a note, and the page reloads its props to list it,
  // with no document load.
  const loads = documentLoads(page);
  const title = `Agent note ${Date.now()}`;
  const created = await agent.invoke('notes.create', { title, body: 'Written through WebMCP.' });
  expect(created.output?.isError, JSON.stringify(created)).toBe(false);
  expect(created.output?.structuredContent).toMatchObject({ title });
  await expect(page.getByRole('link', { name: title })).toBeVisible();
  expect(loads()).toBe(0);

  // The search reads its input from the query the page sends.
  const found = await agent.invoke('notes.search', { query: 'WebMCP', limit: 5 });
  expect(found.output?.isError, JSON.stringify(found)).toBe(false);
  const notes = (found.output?.structuredContent?.notes ?? []) as { title: string }[];
  expect(notes.map((note) => note.title)).toEqual([title]);
});
