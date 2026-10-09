import { Form, Head, Link, usePage } from '@inertiajs/react'

export default function ForgotPassword() {
  // The public root the server shares with every page (`RootShare`): empty
  // at the host root, `/billing` behind a proxy that serves the app there.
  // Every URL this page posts to or links is built from it, so one build
  // runs at both.
  // `types/inertia-props.ts` types it, so `usePage()` takes no argument.
  const { root } = usePage().props

  // The server answers every address the same way and flashes the same
  // toast, which the guest layout shows, so nothing here tells whether the
  // address has an account.
  return (
    <>
      <Head title="Reset your password" />
      <div>
        <h2 className="mt-6 text-center text-3xl font-extrabold text-gray-900">
          Reset your password
        </h2>
        <p className="mt-2 text-center text-sm text-gray-600">
          Enter the email address you verified for your account and we will send you a link
          to choose a new password.
        </p>
      </div>

      <Form action={`${root}/forgot-password`} method="post" className="mt-8 space-y-6">
        {({ errors, processing }) => (
          <>
            <div>
              <label htmlFor="email" className="sr-only">
                Email address
              </label>
              <input
                id="email"
                name="email"
                type="email"
                autoComplete="email"
                required
                className="appearance-none relative block w-full px-3 py-2 border border-gray-300 placeholder-gray-500 text-gray-900 rounded-md focus:outline-none focus:ring-indigo-500 focus:border-indigo-500 focus:z-10 sm:text-sm"
                placeholder="Email address"
              />
              {errors.email && <p className="mt-1 text-sm text-red-600">{errors.email}</p>}
            </div>

            <div>
              <button
                type="submit"
                disabled={processing}
                className="group relative w-full flex justify-center py-2 px-4 border border-transparent text-sm font-medium rounded-md text-white bg-indigo-600 hover:bg-indigo-700 focus:outline-none focus:ring-2 focus:ring-offset-2 focus:ring-indigo-500 disabled:opacity-50"
              >
                {processing ? 'Sending...' : 'Send reset link'}
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
