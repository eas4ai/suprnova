<script setup lang="ts">
import { Link, usePage } from '@inertiajs/vue3'

defineProps<{ counter: number }>()
const root = usePage().props.root
</script>

<template>
  <div>
    <h1>Second</h1>
    <p data-testid="counter">{{ counter }}</p>
    <Link :href="`${root}/agent-fixture/first`" data-testid="to-first">First</Link>
  </div>
</template>
