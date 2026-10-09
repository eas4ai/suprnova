import { Form, Head, Link, usePage } from '@inertiajs/react'
import type { ResetPasswordProps } from '../../types/inertia-props'

export default function ResetPassword({ token }: ResetPasswordProps) {
  // The public root the server shares with every page (`RootShare`): empty
  // at the host root, `/billing` behind a proxy that serves the app there.
  // Every URL this page posts to or links is built from it, so one build
  // runs at both.
  // `types/inertia-props.ts` types it, so `usePage()` takes no argument.
  const { root } = usePage().props

  return (
    <>
      <Head title="Choose a new password" />
      <h2 className="mt-6 text-center text-3xl font-extrabold text-gray-900">
        Choose a new password
      </h2>

      <Form
        action={`${root}/reset-password`}
        method="post"
        resetOnError={['password', 'password_confirmation']}
        className="mt-8 space-y-6"
      >
        {({ errors, processing }) => (
          <>
            {/* The token came in on the mailed link's query string and goes
                back in the form body; the server never reads it from the
                URL on submit. */}
            <input type="hidden" name="token" value={token} />

            {errors.token && (
              <p className="text-center text-sm text-red-600">
                {errors.token}{' '}
                <Link
                  href={`${root}/forgot-password`}
                  className="text-indigo-600 hover:text-indigo-500"
                >
                  Request a new link
                </Link>
              </p>
            )}

            <div className="space-y-4">
              <div>
                <label htmlFor="password" className="block text-sm font-medium text-gray-700">
                  New password
                </label>
                <input
                  id="password"
                  name="password"
                  type="password"
                  autoComplete="new-password"
                  required
                  className="mt-1 block w-full px-3 py-2 border border-gray-300 rounded-md shadow-sm focus:outline-none focus:ring-indigo-500 focus:border-indigo-500 sm:text-sm"
                />
                {errors.password && (
                  <p className="mt-1 text-sm text-red-600">{errors.password}</p>
                )}
              </div>

              <div>
                <label htmlFor="password_confirmation" className="block text-sm font-medium text-gray-700">
                  Confirm new password
                </label>
                <input
                  id="password_confirmation"
                  name="password_confirmation"
                  type="password"
                  autoComplete="new-password"
                  required
                  className="mt-1 block w-full px-3 py-2 border border-gray-300 rounded-md shadow-sm focus:outline-none focus:ring-indigo-500 focus:border-indigo-500 sm:text-sm"
                />
                {errors.password_confirmation && (
                  <p className="mt-1 text-sm text-red-600">{errors.password_confirmation}</p>
                )}
              </div>
            </div>

            <div>
              <button
                type="submit"
                disabled={processing}
                className="w-full flex justify-center py-2 px-4 border border-transparent rounded-md shadow-sm text-sm font-medium text-white bg-indigo-600 hover:bg-indigo-700 focus:outline-none focus:ring-2 focus:ring-offset-2 focus:ring-indigo-500 disabled:opacity-50"
              >
                {processing ? 'Saving...' : 'Save new password'}
              </button>
            </div>
          </>
        )}
      </Form>
      <p className="text-center">
        <Link href={`${root}/login`} className="text-indigo-600 hover:text-indigo-500">
          Back to sign in
        </Link>
      </p>
    </>
  )
}
