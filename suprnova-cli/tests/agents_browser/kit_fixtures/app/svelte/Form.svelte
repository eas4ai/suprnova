<script lang="ts">
  import { useForm, usePage } from '@inertiajs/svelte'
  import { useFormTool } from '../../lib/agents'
  import { toolSchemas } from '../../types/tool-schemas'

  let { notes }: { notes: string[] } = $props()
  const { root } = usePage().props
  const note = useForm<{ title: string; body: string | null }>({ title: '', body: null })
  useFormTool(note, {
    name: 'fixture.notes.create',
    description: 'Write a note on the fixture page.',
    schema: toolSchemas.FixtureNoteRequest,
    action: { method: 'post', url: `${root}/agent-fixture/notes` },
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

<div>
  <h1>Form</h1>
  <form
    onsubmit={(event) => {
      event.preventDefault()
      note.post(`${root}/agent-fixture/notes`)
    }}
  >
    <input bind:value={note.title} data-testid="title" />
    {#if note.errors.title}
      <p data-testid="title-error">{note.errors.title}</p>
    {/if}
    <button type="submit" data-testid="save">Save</button>
  </form>
  <form
    onsubmit={(event) => {
      event.preventDefault()
      purge.post(`${root}/agent-fixture/purge`)
    }}
  >
    <input bind:value={purge.confirm} data-testid="confirm" />
    <button type="submit" data-testid="purge">Purge</button>
  </form>
  <ul data-testid="notes">
    {#each notes as title (title)}
      <li>{title}</li>
    {/each}
  </ul>
</div>
