import { useForm, usePage } from '@inertiajs/react'
import { useFormTool } from '../../lib/agents'
import { toolSchemas } from '../../types/tool-schemas'

export default function Form({ notes }: { notes: string[] }) {
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
  return (
    <div>
      <h1>Form</h1>
      <form
        onSubmit={(event) => {
          event.preventDefault()
          note.post(`${root}/agent-fixture/notes`)
        }}
      >
        <input data-testid="title" value={note.data.title} onChange={(event) => note.setData('title', event.target.value)} />
        {note.errors.title && <p data-testid="title-error">{note.errors.title}</p>}
        <button type="submit" data-testid="save">
          Save
        </button>
      </form>
      <form
        onSubmit={(event) => {
          event.preventDefault()
          purge.post(`${root}/agent-fixture/purge`)
        }}
      >
        <input data-testid="confirm" value={purge.data.confirm} onChange={(event) => purge.setData('confirm', event.target.value)} />
        <button type="submit" data-testid="purge">
          Purge
        </button>
      </form>
      <ul data-testid="notes">
        {notes.map((title) => (
          <li key={title}>{title}</li>
        ))}
      </ul>
    </div>
  )
}
