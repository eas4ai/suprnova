// Localization wrapper around `@fluent/bundle`, driven by the `lang`
// Inertia shared prop (see `LocaleShare` in the Suprnova framework).
//
// `$state` runes, not a store - this needs the `.svelte.ts` extension
// (a plain `.ts` module can't use runes).
//
// Usage, once per navigation (e.g. from `router.on('navigate', ...)` in
// `main.ts`, or once at boot with the initial page):
//
//   import { t, initLang } from './lib/lang.svelte'
//   await initLang($page)
//   t('welcome', { app: 'My App' })

import { FluentBundle, FluentResource } from '@fluent/bundle'
import type { MessageKey } from '../types/lang-keys'

/** Shape of the `lang` prop shared by the framework's `LocaleShare`. */
export interface LangShare {
  locale: string
  fallback: string
  catalog: { url: string; hash: string } | null
}

/**
 * Minimal structural shape of an Inertia page object - matches
 * `@inertiajs/svelte`'s page store value (and its Vue/React equivalents)
 * without importing `@inertiajs/core` directly.
 */
export interface LangPage {
  props: {
    lang?: LangShare
    [key: string]: unknown
  }
}

let locale = $state('en')
let fallbackLocale = $state('en')
let bundle: FluentBundle | null = null

// Bumped on every catalog load/failure. `bundle` stays a plain (non-rune)
// variable - a `FluentBundle` instance doesn't behave correctly wrapped
// in Svelte's reactive Proxy - so `t()` reads this rune to register as a
// reactive dependency for callers inside a component/template.
let catalogVersion = $state(0)

/**
 * Read the `lang` shared prop off `page`, then fetch and parse its
 * Fluent catalog. Safe to call on every Inertia navigation - the
 * `?v=<hash>` cache-buster on `catalog.url` makes a repeat fetch for an
 * unchanged locale a browser cache hit.
 *
 * `catalog` is `null` when the app has no `Translator` bound; in that
 * case `t()` falls back to returning the raw key rather than throwing.
 */
export async function initLang(page: LangPage): Promise<void> {
  const lang = page.props.lang
  if (!lang) {
    return
  }

  locale = lang.locale
  fallbackLocale = lang.fallback

  if (!lang.catalog) {
    bundle = null
    catalogVersion++
    return
  }

  try {
    const response = await fetch(lang.catalog.url)
    if (!response.ok) {
      bundle = null
      catalogVersion++
      return
    }
    const source = await response.text()
    const next = new FluentBundle(lang.locale, { useIsolating: false })
    next.addResource(new FluentResource(source))
    bundle = next
  } catch {
    // Offline, network failure, etc. - degrade to the raw-key fallback
    // below rather than leaving the page unable to render.
    bundle = null
  }
  catalogVersion++
}

/**
 * Translate `key`, formatting `args` into the message's Fluent
 * placeables. When no catalog is loaded (pre-`initLang`, a translator-less
 * app, or a fetch failure) or `key` has no entry, returns `key` itself -
 * a missing translation should be visibly wrong, never a crashed page.
 */
export function t(key: MessageKey, args?: Record<string, string | number>): string {
  void catalogVersion // reactive dependency for template/derived callers
  if (!bundle) {
    return key
  }
  const message = bundle.getMessage(key)
  if (!message?.value) {
    return key
  }
  return bundle.formatPattern(message.value, args, [])
}

/** The active locale, updated by the most recent `initLang` call. */
export function currentLocale(): string {
  return locale
}

/**
 * The configured fallback locale. Exposed as a getter function rather
 * than a plain exported binding - Svelte 5's cross-module `$state`
 * reactivity for primitives is only observed by components that read it
 * through a function call at render time, not a destructured import.
 */
export function currentFallbackLocale(): string {
  return fallbackLocale
}
