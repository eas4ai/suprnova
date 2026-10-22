import { Form, Head, Link, usePage } from '@inertiajs/react'
import type { VerifyEmailProps } from '../../types/inertia-props'

export default function VerifyEmail({ email }: VerifyEmailProps) {
  // The public root the server shares with every page (`RootShare`): empty
  // at the host root, `/billing` behind a proxy that serves the app there.
  // Every URL this page posts to or links is built from it, so one build
  // runs at both.
  // `types/inertia-props.ts` types it, so `usePage()` takes no argument.
  const { root } = usePage().props

  return (
    <>
      <Head title="Verify your email address" />
      <div>
        <h2 className="mt-6 text-center text-3xl font-extrabold text-gray-900">
          Verify your email address
        </h2>
        <p className="mt-2 text-center text-sm text-gray-600">
          We sent a verification link to <strong>{email}</strong>. Open it while signed in
          to this account.
        </p>
      </div>

      {/* The resend has no fields. The server flashes a toast when the link
          is on its way, which the guest layout shows. */}
      <Form
        action={`${root}/email/verification-notification`}
        method="post"
        className="mt-8 space-y-6"
      >
        {({ errors, processing }) => (
          <>
            {Object.entries(errors).map(([field, message]) => (
              <p key={field} className="text-center text-sm text-red-600">
                {message}
              </p>
            ))}
            <div>
              <button
                type="submit"
                disabled={processing}
                className="group relative w-full flex justify-center py-2 px-4 border border-transparent text-sm font-medium rounded-md text-white bg-indigo-600 hover:bg-indigo-700 focus:outline-none focus:ring-2 focus:ring-offset-2 focus:ring-indigo-500 disabled:opacity-50"
              >
                {processing ? 'Sending...' : 'Resend verification link'}
              </button>
            </div>
          </>
        )}
      </Form>
      <p className="text-center">
        <Link href={`${root}/dashboard`} className="text-indigo-600 hover:text-indigo-500">
          Continue to your dashboard
        </Link>
      </p>
    </>
  )
}
