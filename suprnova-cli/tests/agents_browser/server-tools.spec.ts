import { expect, test, type Page } from '@playwright/test';

// AGENT-014: the server tools a Live page offers, registered with WebMCP
// beside its islands' tools and called the way a built-in agent calls them,
// through the DevTools `WebMCP` domain's `invokeTool`. The host's
// `/server-tools` page offers `notes.show` to everyone and `posts.create`
// and `posts.clear` to a signed-in user.

interface ToolResponse {
  readonly status: string;
  readonly output?: {
    readonly structuredContent?: Record<string, unknown>;
    readonly isError?: boolean;
    readonly content?: { text: string }[];
  };
}

interface ToolCall {
  readonly tool: string;
  readonly method: string;
  readonly path: string;
  readonly query: string | null;
  readonly csrf: string | null;
  readonly session: boolean;
  readonly status: number;
}

interface Recorded {
  readonly path: string;
  readonly tool: string | null;
}

interface Descriptor {
  readonly name: string;
  readonly annotations: Record<string, boolean>;
}

const SERVER_TOOLS = ['notes.show', 'posts.clear', 'posts.create'];

async function webmcp(page: Page) {
  const cdp = await page.context().newCDPSession(page);
  await cdp.send('WebMCP.enable');
  const responses = new Map<string, ToolResponse>();
  let frameId = '';
  cdp.on('WebMCP.toolsAdded', (event: { tools: { frameId: string }[] }) => {
    for (const tool of event.tools) frameId = tool.frameId;
  });
  cdp.on('WebMCP.toolResponded', (event: ToolResponse & { invocationId: string }) => {
    responses.set(event.invocationId, event);
  });
  return {
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

async function registered(page: Page): Promise<string[]> {
  return page.evaluate(async () => {
    const context = (document as unknown as { modelContext: { getTools(): Promise<{ name: string }[]> } })
      .modelContext;
    return (await context.getTools()).map((tool) => tool.name).sort();
  });
}

/// The server tools among the registered ones.
async function serverTools(page: Page): Promise<string[]> {
  return (await registered(page)).filter((name) => !name.startsWith('counter.'));
}

async function signIn(page: Page, user: string) {
  await page.goto(`/login/${user}`);
}

async function reset(page: Page) {
  await page.request.get('/test/reset');
}

async function toolCalls(page: Page): Promise<ToolCall[]> {
  const body = (await (await page.request.get('/test/tool-calls')).json()) as { calls: ToolCall[] };
  return body.calls;
}

async function liveRequests(page: Page): Promise<Recorded[]> {
  const body = (await (await page.request.get('/test/requests')).json()) as { requests: Recorded[] };
  return body.requests.filter((request) => request.path === '/__live/action');
}

function posts(page: Page) {
  return page.locator('[data-testid="posts"]');
}

async function openServerTools(page: Page, expected: string[]) {
  const agent = await webmcp(page);
  await page.goto('/server-tools');
  await expect.poll(() => serverTools(page)).toEqual(expected);
  return agent;
}

test('agent_014_live_page_registers_the_server_tools_the_viewer_may_call', async ({ page }) => {
  await reset(page);
  // A guest sees the public tool only.
  await openServerTools(page, ['notes.show']);
  // A signed-in user sees the user tools too, and the counter's tool.
  await signIn(page, 'alice');
  await openServerTools(page, SERVER_TOOLS);
  expect(await registered(page)).toContain('counter.add.counter');
});

test('agent_014_live_page_tools_leave_on_navigation', async ({ page }) => {
  await signIn(page, 'alice');
  await openServerTools(page, SERVER_TOOLS);
  await page.locator('#other').click();
  await page.waitForURL('**/other');
  await expect.poll(() => registered(page)).toEqual([]);
});

test('agent_014_live_server_tool_call_carries_session_csrf_and_header', async ({ page }) => {
  await signIn(page, 'alice');
  const agent = await openServerTools(page, SERVER_TOOLS);
  await reset(page);
  const created = await agent.invoke('posts.create', { title: 'Hello' });
  expect(created.status).toBe('Completed');
  expect(created.output?.isError).toBe(false);
  expect(created.output?.structuredContent).toEqual({ id: 1, title: 'Hello', user: 'alice' });
  expect(created.output).not.toHaveProperty('content');
  const shown = await agent.invoke('notes.show', { note: 'n/7' });
  expect(shown.output?.isError).toBe(false);
  expect(shown.output?.structuredContent).toEqual({ note: 'n/7', same: true });
  const [create, show] = await toolCalls(page);
  // The route ran with the page's session and its CSRF token, which the
  // CSRF check accepted, and the header named the tool.
  expect(create).toMatchObject({
    tool: 'posts.create',
    method: 'POST',
    path: '/posts',
    csrf: 'X-XSRF-TOKEN',
    session: true,
    status: 200,
  });
  // A GET tool sends its arguments, its path argument included, in the query.
  expect(show).toMatchObject({
    tool: 'notes.show',
    method: 'GET',
    path: '/notes/n%2F7',
    query: 'note=n%2F7',
    csrf: 'X-XSRF-TOKEN',
    session: true,
    status: 200,
  });
});

test('agent_014_live_write_tool_rerenders_the_mounted_islands', async ({ page }) => {
  await reset(page);
  await signIn(page, 'alice');
  const agent = await openServerTools(page, SERVER_TOOLS);
  await expect(posts(page)).toHaveText('0');
  const created = await agent.invoke('posts.create', { title: 'Hello' });
  expect(created.output?.isError).toBe(false);
  // The islands rendered fresh before the call resolved: the screen already
  // shows what the agent did.
  expect(await posts(page).textContent()).toBe('1');
  const renders = await liveRequests(page);
  expect(renders.length).toBeGreaterThanOrEqual(2);
  expect(renders.every((request) => request.tool === null)).toBe(true);
  const cleared = await agent.invoke('posts.clear', {});
  expect(cleared.output?.isError).toBe(false);
  expect(await posts(page).textContent()).toBe('0');
});

test('agent_014_live_read_only_tool_does_not_rerender', async ({ page }) => {
  await signIn(page, 'alice');
  const agent = await openServerTools(page, SERVER_TOOLS);
  await reset(page);
  const shown = await agent.invoke('notes.show', { note: '1' });
  expect(shown.output?.isError).toBe(false);
  await page.waitForTimeout(200);
  expect(await liveRequests(page)).toEqual([]);
});

test('agent_014_live_server_tool_annotations_follow_the_markings', async ({ page }) => {
  // Record what the page passes `registerTool`: Chromium keeps two of the
  // three hints, so the third is read here.
  await page.addInitScript(() => {
    const recorded: unknown[] = [];
    (window as unknown as { __registered: unknown[] }).__registered = recorded;
    const context = (document as unknown as { modelContext?: Record<string, unknown> }).modelContext;
    if (context === undefined) return;
    const original = context['registerTool'] as (tool: unknown, options?: unknown) => unknown;
    Object.defineProperty(context, 'registerTool', {
      configurable: true,
      value(tool: { name: string; annotations: Record<string, boolean> }, options?: unknown) {
        recorded.push({ name: tool.name, annotations: { ...tool.annotations } });
        return original.call(context, tool, options);
      },
    });
  });
  await signIn(page, 'alice');
  await openServerTools(page, SERVER_TOOLS);
  const descriptors = (await page.evaluate(
    () => (window as unknown as { __registered: unknown[] }).__registered,
  )) as Descriptor[];
  const hints = (name: string) => descriptors.find((descriptor) => descriptor.name === name)?.annotations;
  expect(hints('posts.create')).toEqual({
    readOnlyHint: false,
    untrustedContentHint: false,
    consequentialHint: false,
    debugging: false,
  });
  expect(hints('posts.clear')).toEqual({
    readOnlyHint: false,
    untrustedContentHint: false,
    consequentialHint: true,
    debugging: false,
  });
  expect(hints('notes.show')).toEqual({
    readOnlyHint: true,
    untrustedContentHint: true,
    consequentialHint: false,
    debugging: false,
  });
  // What Chromium kept.
  const kept = await page.evaluate(async () => {
    const context = (document as unknown as {
      modelContext: { getTools(): Promise<{ name: string; annotations: Record<string, boolean> }[]> };
    }).modelContext;
    return (await context.getTools()).find((tool) => tool.name === 'notes.show')?.annotations;
  });
  expect(kept).toMatchObject({ readOnlyHint: true, untrustedContentHint: true });
});
