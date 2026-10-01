import { useState } from 'react'
import { useForm } from '@inertiajs/react'
import type { VerifyEmailProps } from '../../types/inertia-props'

export default function VerifyEmail({ email }: VerifyEmailProps) {
  const { post, processing } = useForm({})
  const [sent, setSent] = useState(false)

  const resend = (e: React.FormEvent) => {
    e.preventDefault()
    post('/email/verification-notification', { onSuccess: () => setSent(true) })
  }

  return (
    <div className="min-h-screen flex items-center justify-center bg-gray-50 py-12 px-4 sm:px-6 lg:px-8">
      <div className="max-w-md w-full space-y-8">
        <div>
          <h2 className="mt-6 text-center text-3xl font-extrabold text-gray-900">
            Verify your email address
          </h2>
          <p className="mt-2 text-center text-sm text-gray-600">
            We sent a verification link to <strong>{email}</strong>. Open it while signed in
            to this account.
          </p>
        </div>

        {sent && (
          <p role="status" className="text-center text-sm text-green-700">
            A new link has been sent.
          </p>
        )}

        <form className="mt-8 space-y-6" onSubmit={resend}>
          <div>
            <button
              type="submit"
              disabled={processing}
              className="group relative w-full flex justify-center py-2 px-4 border border-transparent text-sm font-medium rounded-md text-white bg-indigo-600 hover:bg-indigo-700 focus:outline-none focus:ring-2 focus:ring-offset-2 focus:ring-indigo-500 disabled:opacity-50"
            >
              {processing ? 'Sending...' : 'Resend verification link'}
            </button>
          </div>

          <div className="text-center">
            <a href="/dashboard" className="text-indigo-600 hover:text-indigo-500">
              Continue to your dashboard
            </a>
          </div>
        </form>
      </div>
    </div>
  )
}
