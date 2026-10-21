<script setup lang="ts">
import { computed, ref } from 'vue'
import {
  Deferred,
  Head,
  Link,
  router,
  setLayoutProps,
  useHttp,
  usePage,
  usePoll,
  WhenVisible,
} from '@inertiajs/vue3'
import type { DashboardProps, UserInfo } from '../types/inertia-props'

// The public root the server shares with every page (`RootShare`): empty
// at the host root, `/billing` behind a proxy that serves the app there.
// Every URL this page posts to or links is built from it, so one build
// runs at both.
// `types/inertia-props.ts` types it, so `usePage()` takes no argument.
const { root } = usePage().props

// The signed-in user, shared with every page as `auth.user` and read from
// the page so a reload of `auth` shows here. The dashboard sits behind the
// `auth` middleware, so it is never `null` here in practice.
const page = usePage()
const user = computed(() => page.props.auth.user)

// Neither prop is on the first response: `stats` is deferred, which the
// client fetches right after the page loads, and `recent_notes` is
// optional, which the server sends only when a visit asks for it by name.
// So both are optional here, and the template waits for them.
defineProps<Partial<DashboardProps>>()

// The application layout shows this as the page's heading. The next visit
// to a page that sets no heading clears it.
setLayoutProps({ heading: 'Dashboard' })

// Reload `stats` alone every ten seconds while this page is open; the poll
// stops when the page unmounts.
usePoll(10000, { only: ['stats'] })

// The display-name form posts JSON to `POST /profile/name` through
// `useHttp`, outside Inertia's page visits. The shown name lives in the
// form's `name` field. `optimistic` sets it to the draft before the
// request goes out; a 422 restores the field to what it was and fills
// `errors.name` from the response's `errors`
// (`packages/vue3/src/useHttp.ts`). Any other failure restores the field
// too and rejects; the promise goes back to Vue, whose error handler
// reports it.
const profile = useHttp<{ name: string }, { user: UserInfo }>({ name: user.value?.name ?? '' })
const draft = ref(user.value?.name ?? '')

function saveName() {
  return profile.optimistic(() => ({ name: draft.value })).post(`${root}/profile/name`, {
    // Reload the shared `auth` prop, so the layout shows the saved name.
    onSuccess: () => router.reload({ only: ['auth'] }),
  })
}
</script>

<template>
  <Head title="Dashboard" />

  <div class="space-y-6">
    <!-- Tall on purpose: recent notes start out of view, so
         `WhenVisible` loads them only after the visitor scrolls. -->
    <div class="min-h-screen space-y-6">
      <section class="rounded-lg bg-white p-6 shadow">
        <h2 class="text-lg font-medium text-gray-900">Welcome, {{ profile.name }}!</h2>
        <p v-if="user" class="mt-1 text-sm text-gray-500">Signed in as {{ user.email }}.</p>

        <form class="mt-4 flex flex-wrap items-start gap-3" @submit.prevent="saveName">
          <div>
            <label for="name" class="sr-only">Display name</label>
            <input
              id="name"
              v-model="draft"
              name="name"
              type="text"
              required
              class="block w-64 rounded-md border border-gray-300 px-3 py-2 text-sm shadow-sm focus:border-indigo-500 focus:outline-none focus:ring-indigo-500"
            />
            <p v-if="profile.errors.name" class="mt-1 text-sm text-red-600">
              {{ profile.errors.name }}
            </p>
          </div>
          <button
            type="submit"
            :disabled="profile.processing"
            class="rounded-md bg-indigo-600 px-4 py-2 text-sm font-medium text-white hover:bg-indigo-700 disabled:opacity-50"
          >
            {{ profile.processing ? 'Saving...' : 'Save name' }}
          </button>
        </form>
      </section>

      <section class="grid gap-4 sm:grid-cols-2">
        <Deferred data="stats">
          <template #fallback>
            <div class="rounded-lg bg-white p-6 text-sm text-gray-500 shadow">Counting your notes...</div>
            <div class="rounded-lg bg-white p-6 text-sm text-gray-500 shadow">Counting today's notes...</div>
          </template>

          <div class="rounded-lg bg-white p-6 shadow">
            <p class="text-sm text-gray-500">Notes</p>
            <p class="mt-1 text-3xl font-semibold text-gray-900">{{ stats?.notes }}</p>
          </div>
          <div class="rounded-lg bg-white p-6 shadow">
            <p class="text-sm text-gray-500">Written today</p>
            <p class="mt-1 text-3xl font-semibold text-gray-900">{{ stats?.written_today }}</p>
          </div>
        </Deferred>
      </section>

    </div>

    <section class="rounded-lg bg-white p-6 shadow">
      <div class="flex items-center justify-between">
        <h2 class="text-lg font-medium text-gray-900">Recent notes</h2>
        <Link :href="`${root}/notes`" class="text-sm text-indigo-600 hover:text-indigo-500">All notes</Link>
      </div>

      <!-- Below the fold: `recent_notes` loads when this scrolls into view. -->
      <WhenVisible data="recent_notes">
        <template #fallback>
          <p class="mt-4 text-sm text-gray-500">Loading your recent notes...</p>
        </template>

        <ul v-if="recent_notes?.length" class="mt-4 divide-y divide-gray-100">
          <li v-for="note in recent_notes" :key="note.id" class="py-2">
            <Link :href="`${root}/notes/${note.id}`" class="text-sm text-gray-900 hover:text-indigo-600">
              {{ note.title }}
            </Link>
          </li>
        </ul>
        <p v-else class="mt-4 text-sm text-gray-500">You have not written a note yet.</p>
      </WhenVisible>
    </section>
  </div>
</template>
