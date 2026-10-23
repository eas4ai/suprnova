// The application's name, which ends every tab title. The server titles a
// page that sets no title of its own with the same name
// (`.default_title(...)` in `src/bootstrap.rs`); change both together.
export const appName = '{project_title}'

/**
 * The tab title for a page titled `title`: `Notes - My App`, or the
 * application's name alone for a page without a title.
 */
export function pageTitle(title: string): string {
  return title ? `${title} - ${appName}` : appName
}
