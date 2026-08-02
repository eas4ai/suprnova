// Localization wrapper around `@fluent/bundle`, driven by the `lang`
// Inertia shared prop (see `LocaleShare` in the Suprnova framework).
//
// Wrap the app once with `<LangProvider>` and call `initLang` per
// navigation (e.g. from `router.on('navigate', ...)` in `main.tsx`, or
// once at boot with the initial page):
//
//   import { LangProvider, useLang } from './lib/lang'
//   const { t, locale, initLang } = useLang()
//   await initLang(usePage())
//   t('welcome', { app: 'My App' })
//
// Plain `.ts` (not `.tsx`) — the provider is built with `createElement`
// so this file never needs JSX syntax.

import {
  createContext,
  createElement,
  useContext,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from 'react'
import { FluentBundle, FluentResource } from '@fluent/bundle'
import type { MessageKey } from '../types/lang-keys'

/** Shape of the `lang` prop shared by the framework's `LocaleShare`. */
export interface LangShare {
  locale: string
  fallback: string
  catalog: { url: string; hash: string } | null
}

/**
 * Minimal structural shape of an Inertia page object — matches
 * `@inertiajs/react`'s `Page<T>` (and its Vue/Svelte equivalents)
 * without importing `@inertiajs/core` directly.
 */
export interface LangPage {
  props: {
    lang?: LangShare
    [key: string]: unknown
  }
}

interface LangContextValue {
  locale: string
  fallbackLocale: string
  t: (key: MessageKey, args?: Record<string, string | number>) => string
  currentLocale: () => string
  initLang: (page: LangPage) => Promise<void>
}

const LangContext = createContext<LangContextValue | null>(null)

/** Wrap the app (or its root layout) once to make `useLang()` available. */
export function LangProvider({ children }: { children: ReactNode }) {
  const [locale, setLocale] = useState('en')
  const [fallbackLocale, setFallbackLocale] = useState('en')
  const bundleRef = useRef<FluentBundle | null>(null)
  // Bumped on every catalog load/failure so `t()` closures captured by
  // the memoized context value are rebuilt — the bundle itself lives in
  // a ref rather than state, since swapping it into React state on every
  // fetch would be heavier than this counter.
  const [catalogVersion, setCatalogVersion] = useState(0)

  const initLang = async (page: LangPage): Promise<void> => {
    const lang = page.props.lang
    if (!lang) {
      return
    }
    setLocale(lang.locale)
    setFallbackLocale(lang.fallback)

    if (!lang.catalog) {
      bundleRef.current = null
      setCatalogVersion((v) => v + 1)
      return
    }

    try {
      const response = await fetch(lang.catalog.url)
      if (!response.ok) {
        bundleRef.current = null
        setCatalogVersion((v) => v + 1)
        return
      }
      const source = await response.text()
      const next = new FluentBundle(lang.locale, { useIsolating: false })
      next.addResource(new FluentResource(source))
      bundleRef.current = next
    } catch {
      // Offline, network failure, etc. — degrade to the raw-key fallback
      // in `t()` rather than leaving the page unable to render.
      bundleRef.current = null
    }
    setCatalogVersion((v) => v + 1)
  }

  const value = useMemo<LangContextValue>(() => {
    const t = (key: MessageKey, args?: Record<string, string | number>): string => {
      const bundle = bundleRef.current
      if (!bundle) {
        return key
      }
      const message = bundle.getMessage(key)
      if (!message?.value) {
        return key
      }
      return bundle.formatPattern(message.value, args, [])
    }
    return { locale, fallbackLocale, t, currentLocale: () => locale, initLang }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [locale, fallbackLocale, catalogVersion])

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
