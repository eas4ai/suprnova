import { xhrHttpClient } from '@inertiajs/core'
import { createStaticClient, readStaticPublication } from './core'

/** The React entry installs this before Inertia makes its first request. */
export function staticHttpClient() {
  return createStaticClient(xhrHttpClient, readStaticPublication(document))
}
