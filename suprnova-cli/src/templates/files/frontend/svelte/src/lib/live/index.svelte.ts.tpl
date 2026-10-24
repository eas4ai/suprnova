import { router, usePage } from '@inertiajs/svelte'
import { untrack } from 'svelte'
import {
  browserLiveTransport,
  startInertiaLive,
  type InertiaLivePage,
  type InertiaLiveRouter,
  type InertiaLiveState,
} from './core'

/** Reactive state for this component's Live connection. */
export type LiveState = {
  readonly connected: boolean
  readonly status: InertiaLiveState
}

/** Connect this component to the Live topics rendered by the server. */
export function useLive(): LiveState {
  const page = usePage()
  let status = $state<InertiaLiveState>('disconnected')

  $effect(() => {
    const transport = browserLiveTransport()
    if (transport === null) {
      status = 'degraded'
      return
    }

    status = 'connecting'
    // Setup reads the reactive Inertia page synchronously. Keep that read out
    // of this effect so a partial reload does not tear down the connection.
    const connection = untrack(() =>
      startInertiaLive({
        getPage: () => page as unknown as InertiaLivePage,
        onState: (_topic, next) => {
          status = next
        },
        router: router as unknown as InertiaLiveRouter,
        transport,
      }),
    )
    return () => {
      connection.close()
      status = 'closed'
    }
  })

  return {
    get connected() {
      return status === 'current'
    },
    get status() {
      return status
    },
  }
}
