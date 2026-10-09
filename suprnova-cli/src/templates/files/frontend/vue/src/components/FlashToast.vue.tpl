<script setup lang="ts">
import { computed } from 'vue'
import { usePage } from '@inertiajs/vue3'

// The server flashes a toast with `Inertia::flash("toast", Toast { .. })`
// after a sign-in, a registration, a saved note and the account flows. It
// arrives in the page's `flash`, on the one page response that follows,
// and the next page arrives without it. So the toast is rendered straight
// from the page and never copied anywhere: it shows once, on the page it
// was flashed for, and is gone after the next visit.
const page = usePage()

// `flash` is typed by the `Flash` struct, the generated `flashDataType`:
// `Inertia::flash("toast", Toast { .. })` fills its `toast`.
const toast = computed(() => page.flash.toast)

const tone = computed(() => {
  const kind = toast.value?.kind
  if (kind === 'error') {
    return 'border-red-200 bg-red-50 text-red-800'
  }
  if (kind === 'info') {
    return 'border-sky-200 bg-sky-50 text-sky-800'
  }
  return 'border-green-200 bg-green-50 text-green-800'
})
</script>

<template>
  <div v-if="toast" class="mx-auto mt-4 max-w-7xl px-4 sm:px-6 lg:px-8">
    <p
      role="status"
      :class="['rounded-md border px-4 py-3 text-sm font-medium', tone]"
    >
      {{ toast.message }}
    </p>
  </div>
</template>
