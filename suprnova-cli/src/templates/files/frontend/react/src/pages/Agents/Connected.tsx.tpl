import { Form, Head, usePage } from '@inertiajs/react'
import type { AgentView, ConnectedAgentsProps } from '../../types/inertia-props'

// The apps (MCP clients the user approved) and the agents (registrations
// the user claimed) that act for the signed-in user, newest first, each
// with a revoke button. `controllers::agents::connected` renders it.
//
// A client chose its name, so the name and the host sit in `<bdi>`. An
// agent's provider is the application's trust list entry. Revoking ends
// the access and every token at once.

/** How an agent registered, in words. */
function registeredHow(agent: AgentView): string {
  switch (agent.registration_type) {
    case 'anonymous':
      return 'Registered on its own and claimed by you'
    case 'service_auth':
      return 'Registered with your email address'
    case 'identity_assertion':
      return 'Signed in through a provider you use'
    default:
      return 'An agent account'
  }
}

/** The calendar day of a timestamp the server sent. */
function day(value: string): string {
  return value.slice(0, 10)
}

export default function AgentsConnected({ clients, agents }: ConnectedAgentsProps) {
  // The public root the server shares with every page (`RootShare`): empty
  // at the host root, `/billing` behind a proxy that serves the app there.
  // Every URL this page posts to is built from it, so one build runs at both.
  // `types/inertia-props.ts` types it, so `usePage()` takes no argument.
  const { root } = usePage().props

  return (
    <div className="space-y-6">
      <Head title="Connected agents" />

      <section className="rounded-lg bg-white p-6 shadow">
        <h2 className="text-lg font-semibold text-gray-900">Apps</h2>
        {clients.length === 0 ? (
          <p className="mt-2 text-sm text-gray-500">No apps can act for you.</p>
        ) : (
          <ul className="mt-4 divide-y divide-gray-100">
            {clients.map((client) => (
              <li key={client.id} className="flex items-start justify-between gap-4 py-3">
                <div className="text-sm text-gray-700">
                  <p className="font-medium text-gray-900">
                    <bdi>{client.client_name}</bdi>
                  </p>
                  <p>
                    From <bdi>{client.client_host}</bdi>. Allowed {day(client.created_at)}, last used{' '}
                    {client.last_used_at === null ? 'never' : day(client.last_used_at)}.
                  </p>
                  {client.scopes.length > 0 && (
                    <ul className="mt-1 list-disc pl-5">
                      {client.scopes.map((scope) => (
                        <li key={scope.name}>
                          {scope.description} (<code>{scope.name}</code>)
                        </li>
                      ))}
                    </ul>
                  )}
                </div>
                <Form action={`${root}/connected-agents/clients/${encodeURIComponent(client.id)}/revoke`} method="post">
                  {({ processing }) => (
                    <button
                      type="submit"
                      disabled={processing}
                      className="rounded-md border border-red-300 px-3 py-1 text-sm text-red-700 hover:bg-red-50 disabled:opacity-50"
                    >
                      Revoke
                    </button>
                  )}
                </Form>
              </li>
            ))}
          </ul>
        )}
      </section>

      <section className="rounded-lg bg-white p-6 shadow">
        <h2 className="text-lg font-semibold text-gray-900">Agents</h2>
        {agents.length === 0 ? (
          <p className="mt-2 text-sm text-gray-500">No agents can act for you.</p>
        ) : (
          <ul className="mt-4 divide-y divide-gray-100">
            {agents.map((agent) => (
              <li key={agent.id} className="flex items-start justify-between gap-4 py-3">
                <div className="text-sm text-gray-700">
                  <p className="font-medium text-gray-900">
                    {agent.provider === null ? 'Agent' : `Agent through ${agent.provider}`}
                  </p>
                  <p>
                    {registeredHow(agent)}. Registered {day(agent.created_at)}
                    {agent.claimed_at !== null && `, claimed ${day(agent.claimed_at)}`}.
                  </p>
                  {agent.scopes.length > 0 && (
                    <ul className="mt-1 list-disc pl-5">
                      {agent.scopes.map((scope) => (
                        <li key={scope.name}>
                          {scope.description} (<code>{scope.name}</code>)
                        </li>
                      ))}
                    </ul>
                  )}
                </div>
                <Form action={`${root}/connected-agents/agents/${encodeURIComponent(agent.id)}/revoke`} method="post">
                  {({ processing }) => (
                    <button
                      type="submit"
                      disabled={processing}
                      className="rounded-md border border-red-300 px-3 py-1 text-sm text-red-700 hover:bg-red-50 disabled:opacity-50"
                    >
                      Revoke
                    </button>
                  )}
                </Form>
              </li>
            ))}
          </ul>
        )}
      </section>
    </div>
  )
}
