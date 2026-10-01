import { useForm } from '@inertiajs/react'
import type { ResetPasswordProps } from '../../types/inertia-props'

export default function ResetPassword({ token }: ResetPasswordProps) {
  // The token came in on the mailed link's query string and goes back in
  // the form body; the server never reads it from the URL on submit.
  const { data, setData, post, processing, errors, reset } = useForm({
    token,
    password: '',
    password_confirmation: '',
  })

  const submit = (e: React.FormEvent) => {
    e.preventDefault()
    post('/reset-password', {
      onFinish: () => reset('password', 'password_confirmation'),
    })
  }

  return (
    <div className="min-h-screen flex items-center justify-center bg-gray-50 py-12 px-4 sm:px-6 lg:px-8">
      <div className="max-w-md w-full space-y-8">
        <div>
          <h2 className="mt-6 text-center text-3xl font-extrabold text-gray-900">
            Choose a new password
          </h2>
        </div>

        {errors.token && (
          <p className="text-center text-sm text-red-600">
            {errors.token}{' '}
            <a href="/forgot-password" className="text-indigo-600 hover:text-indigo-500">
              Request a new link
            </a>
          </p>
        )}

        <form className="mt-8 space-y-6" onSubmit={submit}>
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
                value={data.password}
                onChange={(e) => setData('password', e.target.value)}
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
                value={data.password_confirmation}
                onChange={(e) => setData('password_confirmation', e.target.value)}
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

          <div className="text-center">
            <a href="/login" className="text-indigo-600 hover:text-indigo-500">
              Back to sign in
            </a>
          </div>
        </form>
      </div>
    </div>
  )
}
