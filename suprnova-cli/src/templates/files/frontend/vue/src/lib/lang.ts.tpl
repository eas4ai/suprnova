// Localization wrapper around `@fluent/bundle`, driven by the `lang`
// Inertia shared prop (see `LocaleShare` in the Suprnova framework).
//
// Usage, once per navigation (e.g. from `router.on('navigate', ...)` in
// `main.ts`, or once at boot with the initial page):
//
//   import { useLang } from './lib/lang'
//   const { t, locale, initLang } = useLang()
//   await initLang(usePage())
//   t('welcome', { app: 'My App' })

import { ref, type Ref } from 'vue'
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
 * `@inertiajs/vue3`'s `Page<T>` (and its React/Svelte equivalents)
 * without importing `@inertiajs/core` directly.
 */
export interface LangPage {
  props: {
    lang?: LangShare
    [key: string]: unknown
  }
}

const locale: Ref<string> = ref('en')
const fallbackLocale: Ref<string> = ref('en')
let bundle: FluentBundle | null = null

// Bumped on every successful/failed catalog load. `bundle` itself lives
// outside Vue's reactivity system (a `FluentBundle` instance doesn't
// survive being wrapped in a reactive Proxy), so `t()` reads this ref to
// register as a dependency - components calling `t()` inside a template
// or `computed()` re-evaluate when the catalog changes.
const catalogVersion: Ref<number> = ref(0)

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

  locale.value = lang.locale
  fallbackLocale.value = lang.fallback

  if (!lang.catalog) {
    bundle = null
    catalogVersion.value++
    return
  }

  try {
    const response = await fetch(lang.catalog.url)
    if (!response.ok) {
      bundle = null
      catalogVersion.value++
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
  catalogVersion.value++
}

/**
 * Translate `key`, formatting `args` into the message's Fluent
 * placeables. When no catalog is loaded (pre-`initLang`, a translator-less
 * app, or a fetch failure) or `key` has no entry, returns `key` itself -
 * a missing translation should be visibly wrong, never a crashed page.
 */
export function t(key: MessageKey, args?: Record<string, string | number>): string {
  void catalogVersion.value // reactive dependency for template/computed callers
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
  return locale.value
}

/** Vue composable exposing the module-level lang state as refs. */
export function useLang() {
  return { locale, fallbackLocale, t, currentLocale, initLang }
}
