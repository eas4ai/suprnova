import { Link, usePage } from '@inertiajs/react'
import { useState } from 'react'
import AgentFixtureWidget from '../../components/AgentFixtureWidget'

export default function First({ counter }: { counter: number }) {
  const { root } = usePage().props
  const [shown, setShown] = useState(true)
  return (
    <div>
      <h1>First</h1>
      <p data-testid="counter">{counter}</p>
      <button type="button" data-testid="toggle" onClick={() => setShown(!shown)}>
        Toggle the widget
      </button>
      {shown && <AgentFixtureWidget />}
      <Link href={`${root}/agent-fixture/second`} data-testid="to-second">
        Second
      </Link>
    </div>
  )
}
