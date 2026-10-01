import { useState } from 'react'
import { useForm } from '@inertiajs/react'

export default function ForgotPassword() {
  const { data, setData, post, processing, errors } = useForm({
    email: '',
  })

  // The server answers every address the same way, so "sent" is a fact
  // about this submission, not about the account.
  const [sent, setSent] = useState(false)

  const submit = (e: React.FormEvent) => {
    e.preventDefault()
    post('/forgot-password', { onSuccess: () => setSent(true) })
  }

  return (
    <div className="min-h-screen flex items-center justify-center bg-gray-50 py-12 px-4 sm:px-6 lg:px-8">
      <div className="max-w-md w-full space-y-8">
        <div>
          <h2 className="mt-6 text-center text-3xl font-extrabold text-gray-900">
            Reset your password
          </h2>
          <p className="mt-2 text-center text-sm text-gray-600">
            Enter the email address you verified for your account and we will send you a link
            to choose a new password.
          </p>
        </div>

        {sent && (
          <p role="status" className="text-center text-sm text-green-700">
            If that address belongs to a verified account, a reset link is on its way.
          </p>
        )}

        <form className="mt-8 space-y-6" onSubmit={submit}>
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
              value={data.email}
              onChange={(e) => setData('email', e.target.value)}
            />
            {errors.email && (
              <p className="mt-1 text-sm text-red-600">{errors.email}</p>
            )}
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
