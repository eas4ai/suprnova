<script setup lang="ts">
import { computed } from 'vue'
import { Link, usePage } from '@inertiajs/vue3'

// The public root the server shares with every page (`RootShare`): empty
// at the host root, `/billing` behind a proxy that serves the app there.
// Every URL here is built from it, so one build runs at both.
// `types/inertia-props.ts` types it, so `usePage()` takes no argument.
const page = usePage()
const { root } = page.props

// The signed-in user the server shares with every page as `auth.user`, or
// `null` for a guest. It is read from the page on every render, so a
// sign-in, a sign-out or a reload of `auth` shows at once, in whichever
// layout this sits.
const user = computed(() => page.props.auth.user)
</script>

<template>
  <template v-if="user">
    <span class="text-sm text-gray-700">{{ user.name }}</span>
    <!--
      A link that posts keeps the page's state by default
      (`packages/vue3/src/link.ts`), and the layout props, such as the
      dashboard's heading, would follow the visitor to the sign-in page.
      The sign-out starts the next page afresh.
    -->
    <Link
      :href="`${root}/logout`" method="post" as="button" :preserve-state="false"
      class="rounded-md px-3 py-2 text-sm font-medium text-gray-500 hover:text-gray-700"
    >
      Sign out
    </Link>
  </template>
  <template v-else>
    <Link :href="`${root}/login`" class="text-sm font-medium text-indigo-600 hover:text-indigo-500">
      Sign in
    </Link>
    <Link :href="`${root}/register`" class="text-sm font-medium text-indigo-600 hover:text-indigo-500">
      Register
    </Link>
  </template>
</template>
