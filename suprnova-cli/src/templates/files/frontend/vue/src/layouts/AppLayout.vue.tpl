<script setup lang="ts">
import { computed } from 'vue'
import { Link, usePage } from '@inertiajs/vue3'
import AccountLinks from '../components/AccountLinks.vue'
import FlashToast from '../components/FlashToast.vue'

// The public root the server shares with every page (`RootShare`): empty
// at the host root, `/billing` behind a proxy that serves the app there.
// Every URL this layout links is built from it, so one build runs at both.
// `types/inertia-props.ts` types it, so `usePage()` takes no argument.
const page = usePage()
const { root } = page.props

// The application's own pages are for a signed-in user; a guest on the
// home or error page sees only the sign-in and register links.
const signedIn = computed(() => page.props.auth.user !== null)

// The application layout wraps every page outside `auth/` (see the
// `layout` option in `main.ts`). It receives the layout props a page sets
// with `setLayoutProps`, so `heading` is here on the dashboard. A visit to
// a page that sets no heading clears it.
defineProps<{
  heading?: string
}>()
</script>

<template>
  <div class="min-h-screen bg-gray-100">
    <nav class="bg-white shadow">
      <div class="mx-auto flex h-16 max-w-7xl items-center justify-between px-4 sm:px-6 lg:px-8">
        <div class="flex items-center gap-6">
          <template v-if="signedIn">
            <Link :href="`${root}/dashboard`" class="text-sm font-medium text-gray-700 hover:text-gray-900">
              Dashboard
            </Link>
            <Link :href="`${root}/notes`" class="text-sm font-medium text-gray-700 hover:text-gray-900">
              Notes
            </Link>
          </template>
        </div>
        <div class="flex items-center gap-4">
          <AccountLinks />
        </div>
      </div>
    </nav>

    <FlashToast />

    <main class="mx-auto max-w-7xl px-4 py-6 sm:px-6 lg:px-8">
      <h1 v-if="heading" class="mb-6 text-2xl font-semibold text-gray-900">{{ heading }}</h1>
      <slot />
    </main>
  </div>
</template>
