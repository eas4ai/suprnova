// What every page shares, for the browser entry (`main.tsx`) and the server
// render (`ssr.tsx`) alike. Both pass these to `createInertiaApp`, so the
// server's markup and the browser's first render name the same title and
// wrap the page in the same layout, which hydration requires.

import AppLayout from '../layouts/AppLayout'
import GuestLayout from '../layouts/GuestLayout'

/**
 * The application's name, shown in the browser tab after a page's own
 * title. `suprnova new` wrote the project's title here and into
 * `.default_title(..)` in `src/bootstrap.rs`; change both together.
 */
export const appName = '{project_title}'

/**
 * The tab title for a page that sets `<Head title="...">`: the page's title,
 * then the application's name. A page that sets none shows the name alone.
 */
export function title(title: string): string {
  return title ? `${title} - ${appName}` : appName
}

/**
 * The layout a page renders in. The pages under `auth/` share the guest
 * layout, every other page the application layout.
 *
 * Given to `createInertiaApp` rather than rendered by each page, the layout
 * is a persistent layout: a visit between two pages with the same layout
 * swaps the page inside it and keeps the layout mounted, with its state
 * (`packages/react/src/App.ts` in Inertia 3.8.0 renders the layout around
 * the page and keys only the page).
 */
export function layout(name: string) {
  return name.startsWith('auth/') ? GuestLayout : AppLayout
}
