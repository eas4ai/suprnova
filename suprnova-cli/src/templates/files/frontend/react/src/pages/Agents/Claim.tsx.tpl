import { Form, Head, usePage } from '@inertiajs/react'
import type { ClaimProps } from '../../types/inertia-props'

// The page an agent sends the user to: who the agent will act as, with
// which permissions, and the form that takes the code the agent gave the
// user. `controllers::agents::claim` renders it.
//
// The user's address and the address the agent named sit in `<bdi>`, as
// any text someone other than this application chose. The provider's name
// is the application's trust list entry, never one the provider sent. The
// page offers no tool to an agent in the browser: consent comes from the
// person.
//
// A wrong code comes back as the code field's error, with the attempts
// left.
export default function AgentsClaim({ claim, claim_attempt_token }: ClaimProps) {
  // The public root the server shares with every page (`RootShare`): empty
  // at the host root, `/billing` behind a proxy that serves the app there.
  // Every URL this page posts to is built from it, so one build runs at both.
  // `types/inertia-props.ts` types it, so `usePage()` takes no argument.
  const { root } = usePage().props
  const minutes = Math.ceil(claim.expires_in_seconds / 60)

  return (
    <div className="mx-auto max-w-2xl space-y-4 rounded-lg bg-white p-6 shadow">
      <Head title="Let an agent act for you?" />
      <h2 className="text-xl font-semibold text-gray-900">Let an agent act for you?</h2>
      <p className="text-sm text-gray-700">
        {claim.provider !== null ? (
          <>
            An agent signed in through <strong>{claim.provider}</strong> asks to act as you.
          </>
        ) : claim.registration_type === 'service_auth' ? (
          'An agent that named your email address asks to act as you.'
        ) : (
          'An agent that registered on its own asks to act as you.'
        )}
      </p>
      <p className="text-sm text-gray-700">
        If you allow it, the agent will use this application as you, with the permissions below,
        until you revoke it.
      </p>
      <p className="text-sm text-gray-700">
        You are signed in as <strong><bdi>{claim.signed_in_as}</bdi></strong>.
        {claim.email !== null && (
          <>
            {' '}The agent named <strong><bdi>{claim.email}</bdi></strong>.
          </>
        )}
      </p>
      {claim.first_time_account && (
        <p className="rounded-md bg-yellow-50 p-3 text-sm text-yellow-800">
          <strong>Warning:</strong> no agent has acted for your account before. Allow it only if
          you asked an agent to connect.
        </p>
      )}
      {claim.first_time_provider && (
        <p className="rounded-md bg-yellow-50 p-3 text-sm text-yellow-800">
          <strong>Warning:</strong> no agent from this provider has acted for you before.
        </p>
      )}
      {claim.scopes.length === 0 ? (
        <p className="text-sm text-gray-700">It asks for no permissions: it can use only what needs none.</p>
      ) : (
        <div>
          <h3 className="text-sm font-medium text-gray-900">It will be able to</h3>
          <ul className="mt-2 list-disc space-y-1 pl-5 text-sm text-gray-700">
            {claim.scopes.map((scope) => (
              <li key={scope.name}>
                {scope.description} (<code>{scope.name}</code>)
              </li>
            ))}
          </ul>
        </div>
      )}
      <Form action={`${root}/agent/claim`} method="post" resetOnError={['user_code']} className="space-y-3">
        {({ errors, processing }) => (
          <>
            <input type="hidden" name="claim_attempt_token" value={claim_attempt_token} />
            <p className="text-sm text-gray-700">
              Enter the six-digit code the agent gave you. It works for about {minutes} more minutes,{' '}
              {claim.attempts_remaining} wrong entries allowed.
            </p>
            <div>
              <label htmlFor="user_code" className="block text-sm font-medium text-gray-700">
                Code
              </label>
              <input
                id="user_code"
                name="user_code"
                type="text"
                inputMode="numeric"
                autoComplete="one-time-code"
                maxLength={9}
                required
                className="mt-1 block w-40 rounded-md border border-gray-300 px-3 py-2 text-sm shadow-sm focus:border-indigo-500 focus:outline-none focus:ring-indigo-500"
              />
              {errors.user_code && <p className="mt-1 text-sm text-red-600">{errors.user_code}</p>}
            </div>
            <button
              type="submit"
              disabled={processing}
              className="rounded-md bg-indigo-600 px-4 py-2 text-sm font-medium text-white hover:bg-indigo-700 disabled:opacity-50"
            >
              Allow
            </button>
          </>
        )}
      </Form>
      <p className="text-sm text-gray-500">If you did not ask an agent to connect, close this page.</p>
    </div>
  )
}
