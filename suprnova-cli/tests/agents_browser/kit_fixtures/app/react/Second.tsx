import { Link, usePage } from '@inertiajs/react'

export default function Second({ counter }: { counter: number }) {
  const { root } = usePage().props
  return (
    <div>
      <h1>Second</h1>
      <p data-testid="counter">{counter}</p>
      <Link href={`${root}/agent-fixture/first`} data-testid="to-first">
        First
      </Link>
    </div>
  )
}
