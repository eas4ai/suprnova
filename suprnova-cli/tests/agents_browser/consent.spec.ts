import { createHash, randomBytes } from 'node:crypto';
import { createServer } from 'node:http';
import type { AddressInfo } from 'node:net';
import { expect, test, type APIRequestContext, type Page } from '@playwright/test';

// The framework's default approval page in Chromium under the default
// security headers (AGENT-005, AGENT-017), against
// `framework/examples/agents_consent_host.rs`. Run through
// `tests/agents_browser/consent.rs`.

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

/** Register the client dynamically (RFC 7591); its id. */
async function register(request: APIRequestContext, redirectUri: string): Promise<string> {
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

/** Sign the browser in, send it to the authorization endpoint as the client would, and land on the approval page; its response. */
async function openApproval(page: Page, clientId: string, redirectUri: string, baseURL: string) {
  const verifier = randomBytes(32).toString('base64url');
  const challenge = createHash('sha256').update(verifier).digest('base64url');
  const query = new URLSearchParams({
    response_type: 'code',
    client_id: clientId,
    redirect_uri: redirectUri,
    code_challenge: challenge,
    code_challenge_method: 'S256',
    resource: `${baseURL}/mcp`,
    state: 'state-123',
  });
  await page.goto('/login/alice');
  const approval = await page.goto(`/oauth/authorize?${query}`);
  expect(page.url()).toContain('/oauth/approve?authorization=');
  expect(approval?.status()).toBe(200);
  return approval;
}

test('agent_017_default_approval_page_reaches_the_client_under_the_default_security_headers', async ({ page, baseURL }) => {
  const client = await startClient();
  try {
    const clientId = await register(page.request, client.redirectUri);
    const approval = await openApproval(page, clientId, client.redirectUri, baseURL!);
    // The page answers with its own policy, which the global one does not
    // replace: no `form-action` governs its form.
    expect(approval?.headers()['content-security-policy']).toBe("frame-ancestors 'none'");
    expect(approval?.headers()['x-frame-options']).toBe('DENY');
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

test('agent_017_a_form_under_the_global_policy_cannot_reach_the_client', async ({ page, baseURL }) => {
  const client = await startClient();
  try {
    const clientId = await register(page.request, client.redirectUri);
    await openApproval(page, clientId, client.redirectUri, baseURL!);
    const id = new URL(page.url()).searchParams.get('authorization')!;
    // The same form on a page under the application's global policy, the
    // balanced preset.
    const control = await page.goto(`/test/global-policy-approval?authorization=${encodeURIComponent(id)}`);
    expect(control?.headers()['content-security-policy']).toContain("form-action 'self'");
    const pending = `/oauth/authorizations/${encodeURIComponent(id)}`;
    expect((await page.request.get(pending)).status()).toBe(200);
    const refused = page.waitForEvent('console', (message) => message.text().includes('form-action'));
    await page.getByRole('button', { name: 'Allow' }).click();
    // Chromium names the form's own address in the violation, since a
    // report never reveals where a redirect went.
    const message = await refused;
    expect(message.text()).toContain("violates the following Content Security Policy directive: \"form-action 'self'\"");
    expect(message.text()).toContain('blocked');
    // The post reached the server, which recorded the approval and spent
    // the authorization; Chromium refused the redirect that followed. The
    // user approved, and the client never gets the code.
    expect((await page.request.get(pending)).status()).toBe(404);
    expect(client.hits).toHaveLength(0);
  } finally {
    await client.close();
  }
});
