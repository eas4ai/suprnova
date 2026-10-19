<script setup lang="ts">
import { useForm, usePage } from '@inertiajs/vue3'
import { useFormTool } from '../../lib/agents'
import { toolSchemas } from '../../types/tool-schemas'

defineProps<{ notes: string[] }>()
const root = usePage().props.root
const note = useForm<{ title: string; body: string | null }>({ title: '', body: null })
useFormTool(note, {
  name: 'fixture.notes.create',
  description: 'Write a note on the fixture page.',
  schema: toolSchemas.FixtureNoteRequest,
  action: { method: 'post', url: `${root}/agent-fixture/notes` },
})
useFormTool(note, {
  name: 'fixture.notes.create_and_leave',
  description: 'Write a note on the fixture page, then go to the first page.',
  schema: toolSchemas.FixtureNoteRequest,
  action: { method: 'post', url: `${root}/agent-fixture/notes/away` },
})
const purge = useForm<{ confirm: string }>({ confirm: '' })
useFormTool(purge, {
  name: 'fixture.notes.purge',
  description: 'Delete every note on the fixture page.',
  schema: toolSchemas.PurgeRequest,
  action: { method: 'post', url: `${root}/agent-fixture/purge` },
  destructive: true,
})
</script>

<template>
  <div>
    <h1>Form</h1>
    <form @submit.prevent="note.post(`${root}/agent-fixture/notes`)">
      <input v-model="note.title" data-testid="title" />
      <p v-if="note.errors.title" data-testid="title-error">{{ note.errors.title }}</p>
      <button type="submit" data-testid="save">Save</button>
    </form>
    <form @submit.prevent="purge.post(`${root}/agent-fixture/purge`)">
      <input v-model="purge.confirm" data-testid="confirm" />
      <button type="submit" data-testid="purge">Purge</button>
    </form>
    <ul data-testid="notes">
      <li v-for="title in notes" :key="title">{{ title }}</li>
    </ul>
  </div>
</template>
