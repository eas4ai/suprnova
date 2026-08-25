// Localization wrapper around `@fluent/bundle`, driven by the `lang`
// Inertia shared prop (see `LocaleShare` in the Suprnova framework).
//
// `initLang`/`t`/`currentLocale` are module-level, not component state -
// `initLang` has to be callable from `main.tsx`'s `setup()`, before any
// React tree exists, so `await initLang(props.initialPage)` can finish
// loading the catalog before the first render. `<LangProvider>` +
// `useLang()` are a thin reactive subscription over that shared module
// state (via `useSyncExternalStore`, React's own API for this exact
// case - a store that lives outside React and needs to trigger
// re-renders when it changes).
//
// Usage:
//
//   // main.tsx, before mounting:
//   await initLang(props.initialPage)
//   createRoot(el).render(<LangProvider><App {...props} /></LangProvider>)
//
//   // anywhere inside the tree:
//   const { t } = useLang()
//   t('welcome', { app: 'My App' })
//
// Plain `.ts` (not `.tsx`) - `<LangProvider>` is built with
// `createElement` so this file never needs JSX syntax.

import { createContext, createElement, useContext, useSyncExternalStore, type ReactNode } from 'react'
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
 * `@inertiajs/react`'s `Page<T>` (and its Vue/Svelte equivalents)
 * without importing `@inertiajs/core` directly.
 */
export interface LangPage {
  props: {
    lang?: LangShare
    [key: string]: unknown
  }
}

let locale = 'en'
let fallbackLocale = 'en'
let bundle: FluentBundle | null = null
// Bumped on every catalog load/failure and used as the `useSyncExternalStore`
// snapshot - a plain version counter rather than `bundle` itself, so a
// `catalog: null` result (where `bundle` stays `null` before and after)
// still registers as a change and re-renders subscribed components.
let version = 0
const listeners = new Set<() => void>()

function notify(): void {
  version++
  for (const listener of listeners) {
    listener()
  }
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener)
  return () => listeners.delete(listener)
}

/**
 * Fetch and parse the active locale's Fluent catalog from the `lang`
 * Inertia shared prop, and swap it into the module-level `FluentBundle`.
 *
 * `catalog` is `null` when the app has no `Translator` bound (see
 * `LocaleShare`'s doc comment) - in that case `bundle` is cleared and
 * `t()` falls back to returning the raw key, never throwing or blocking
 * render. Never rejects: a fetch failure is caught and degrades the
 * same way.
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
    notify()
    return
  }

  try {
    const response = await fetch(lang.catalog.url)
    if (!response.ok) {
      bundle = null
      notify()
      return
    }
    const source = await response.text()
    const next = new FluentBundle(lang.locale, { useIsolating: false })
    next.addResource(new FluentResource(source))
    bundle = next
  } catch {
    // Offline, network failure, etc. - degrade to the raw-key fallback
    // in `t()` rather than leaving the page unable to render.
    bundle = null
  }
  notify()
}

/**
 * Translate `key` against the currently loaded catalog, formatting any
 * `args` into the message's Fluent placeables.
 *
 * When no catalog is loaded yet (pre-`initLang`, a translator-less app,
 * or a fetch failure) or `key` has no entry in it, this returns `key`
 * itself rather than throwing - a missing translation should be visibly
 * wrong, never a crashed page.
 */
export function t(key: MessageKey, args?: Record<string, string | number>): string {
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

interface LangContextValue {
  locale: string
  fallbackLocale: string
  t: typeof t
  currentLocale: typeof currentLocale
  initLang: typeof initLang
}

const LangContext = createContext<LangContextValue | null>(null)

/** Wrap the app (or its root layout) once to make `useLang()` available. */
export function LangProvider({ children }: { children: ReactNode }) {
  // Re-renders this subtree whenever `notify()` fires (a catalog load
  // completes), so `t()`/`currentLocale()` calls further down reflect
  // the latest state without every component managing its own copy.
  useSyncExternalStore(subscribe, () => version, () => version)
  const value: LangContextValue = { locale, fallbackLocale, t, currentLocale, initLang }
  return createElement(LangContext.Provider, { value }, children)
}

/** Consume the lang context installed by `<LangProvider>`. */
export function useLang(): LangContextValue {
  const ctx = useContext(LangContext)
  if (!ctx) {
    throw new Error('useLang() must be used within a <LangProvider>')
  }
  return ctx
}
