/**
 * The calendar day of a timestamp the server sent, as `2026-10-08`.
 *
 * It is formatted in UTC, so the server-rendered page and the browser that
 * hydrates it print the same text whatever their time zones. A value that
 * does not parse as a date is shown as it was sent.
 */
export function formatDate(value: string): string {
  const date = new Date(value)
  return Number.isNaN(date.getTime()) ? value : date.toISOString().slice(0, 10)
}
