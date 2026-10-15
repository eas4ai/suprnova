import { Head, Link, usePage } from '@inertiajs/react'

// These props come from the framework, not from one of your handlers:
// `InertiaConfig::error_page` in `src/bootstrap.rs` routes every
// framework error response (403, 404, 429, 500, ...) to this page.
// That is why they are declared here: no handler of yours renders
// `Error` with a struct, so `suprnova generate-types` writes no
// `Pages` entry for it in `types/inertia-props.ts`.
interface ErrorProps {
  status: number
  message: string
  request_id?: string
}

export default function ErrorPage({ status, message, request_id }: ErrorProps) {
  // The public root the server shares with every page (`RootShare`): empty
  // at the host root, `/billing` behind a proxy that serves the app there.
  // Every URL this page posts to or links is built from it, so one build
  // runs at both.
  // `types/inertia-props.ts` types it, so `usePage()` takes no argument.
  const { root } = usePage().props

  return (
    <div className="flex items-center justify-center py-12 px-4 sm:px-6 lg:px-8">
      <Head title={String(status)} />
      <div className="max-w-md w-full text-center space-y-4">
        <h1 className="text-6xl font-extrabold text-gray-900">{status}</h1>
        <p className="text-lg text-gray-700">{message}</p>
        {request_id && (
          <p className="text-sm text-gray-500">
            Reference: <code className="bg-gray-100 px-1 rounded">{request_id}</code>
          </p>
        )}
        <p>
          <Link href={`${root}/`} className="text-indigo-600 hover:text-indigo-500">
            Go home
          </Link>
        </p>
      </div>
    </div>
  )
}
