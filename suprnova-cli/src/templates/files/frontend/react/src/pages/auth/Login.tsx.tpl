import { Form, Head, Link, usePage } from '@inertiajs/react'

// Validation errors arrive through the form: a failed submission is a
// `303` back to this page with the errors flashed, and the `Form` component
// hands the page's `errors` to its children. The page itself takes no
// props - declaring an `errors` prop would replace the flashed bag.
export default function Login() {
  // The public root the server shares with every page (`RootShare`): empty
  // at the host root, `/billing` behind a proxy that serves the app there.
  // Every URL this page posts to or links is built from it, so one build
  // runs at both.
  // `types/inertia-props.ts` types it, so `usePage()` takes no argument.
  const { root } = usePage().props

  return (
    <>
      <Head title="Sign in" />
      <h2 className="mt-6 text-center text-3xl font-extrabold text-gray-900">
        Sign in to your account
      </h2>
      {/* `Form` reads the inputs by their `name`. A checked checkbox sends
          the string "on" and an unchecked one nothing, so `transform` turns
          `remember` into the boolean the handler's `LoginRequest` reads. */}
      <Form
        action={`${root}/login`}
        method="post"
        transform={(data) => ({ ...data, remember: Boolean(data.remember) })}
        resetOnError={['password']}
        className="mt-8 space-y-6"
      >
        {({ errors, processing }) => (
          <>
            <div className="rounded-md shadow-sm -space-y-px">
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
                  className="appearance-none rounded-none relative block w-full px-3 py-2 border border-gray-300 placeholder-gray-500 text-gray-900 rounded-t-md focus:outline-none focus:ring-indigo-500 focus:border-indigo-500 focus:z-10 sm:text-sm"
                  placeholder="Email address"
                />
              </div>
              <div>
                <label htmlFor="password" className="sr-only">
                  Password
                </label>
                <input
                  id="password"
                  name="password"
                  type="password"
                  autoComplete="current-password"
                  required
                  className="appearance-none rounded-none relative block w-full px-3 py-2 border border-gray-300 placeholder-gray-500 text-gray-900 rounded-b-md focus:outline-none focus:ring-indigo-500 focus:border-indigo-500 focus:z-10 sm:text-sm"
                  placeholder="Password"
                />
              </div>
            </div>

            {errors.email && <div className="text-red-600 text-sm">{errors.email}</div>}

            {errors.password && <div className="text-red-600 text-sm">{errors.password}</div>}

            <div className="flex items-center">
              <input
                id="remember"
                name="remember"
                type="checkbox"
                className="h-4 w-4 text-indigo-600 focus:ring-indigo-500 border-gray-300 rounded"
              />
              <label htmlFor="remember" className="ml-2 block text-sm text-gray-900">
                Remember me
              </label>
              <Link
                href={`${root}/forgot-password`}
                className="ml-auto text-sm text-indigo-600 hover:text-indigo-500"
              >
                Forgot your password?
              </Link>
            </div>

            <div>
              <button
                type="submit"
                disabled={processing}
                className="group relative w-full flex justify-center py-2 px-4 border border-transparent text-sm font-medium rounded-md text-white bg-indigo-600 hover:bg-indigo-700 focus:outline-none focus:ring-2 focus:ring-offset-2 focus:ring-indigo-500 disabled:opacity-50"
              >
                {processing ? 'Signing in...' : 'Sign in'}
              </button>
            </div>
          </>
        )}
      </Form>
      <p className="text-center">
        <Link href={`${root}/register`} className="text-indigo-600 hover:text-indigo-500">
          Don't have an account? Register
        </Link>
      </p>
    </>
  )
}
