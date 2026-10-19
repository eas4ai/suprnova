<script setup lang="ts">
import { Link, usePage } from '@inertiajs/vue3'
import { ref } from 'vue'
import AgentFixtureWidget from '../../components/AgentFixtureWidget.vue'

defineProps<{ counter: number }>()
const root = usePage().props.root
const shown = ref(true)
</script>

<template>
  <div>
    <h1>First</h1>
    <p data-testid="counter">{{ counter }}</p>
    <button type="button" data-testid="toggle" @click="shown = !shown">Toggle the widget</button>
    <AgentFixtureWidget v-if="shown" />
    <Link :href="`${root}/agent-fixture/second`" data-testid="to-second">Second</Link>
  </div>
</template>
