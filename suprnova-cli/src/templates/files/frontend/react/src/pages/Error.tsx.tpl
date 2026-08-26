// These props come from the framework, not from one of your handlers:
// `InertiaConfig::error_page` in `src/bootstrap.rs` routes every
// framework error response (403, 404, 429, 500, ...) to this page.
// That is why they are declared here rather than imported from
// `types/inertia-props.ts`, which `suprnova generate-types` rewrites
// from your `#[derive(InertiaProps)]` structs.
interface ErrorProps {
  status: number
  message: string
  request_id?: string
}

// The two chrome strings below are deliberately not routed through
// `t()` the way every other page's are. This page renders from the
// error-page middleware, which `Inertia::install` registers ahead of
// `LocaleMiddleware` in `src/bootstrap.rs` - so by the time it renders,
// the request's locale scope has already exited and the `lang` share
// names the app's default catalog, not the visitor's. `t()` here would
// look localized while serving the fallback. Move the call inside the
// locale middleware first if you want this page translated.
export default function ErrorPage({ status, message, request_id }: ErrorProps) {
  return (
    <div className="min-h-screen flex items-center justify-center bg-gray-50 py-12 px-4 sm:px-6 lg:px-8">
      <div className="max-w-md w-full text-center space-y-4">
        <h1 className="text-6xl font-extrabold text-gray-900">{status}</h1>
        <p className="text-lg text-gray-700">{message}</p>
        {request_id && (
          <p className="text-sm text-gray-500">
            Reference: <code className="bg-gray-100 px-1 rounded">{request_id}</code>
          </p>
        )}
        <p>
          <a href="/" className="text-indigo-600 hover:text-indigo-500">
            Go home
          </a>
        </p>
      </div>
    </div>
  )
}
