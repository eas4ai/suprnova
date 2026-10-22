import { router } from '@inertiajs/vue3'
import { getCurrentInstance, onMounted, onUnmounted } from 'vue'
import {
  browserLiveTransport,
  startInertiaLive,
  type InertiaLiveConnection,
  type InertiaLivePage,
  type InertiaLiveRouter,
} from './core'

/** Connect this mounted page to the Live topics rendered by the server. */
export function useLive(): void {
  // Vue's usePage exposes only Inertia's standard keys. The public $page
  // property retains the server's top-level liveBindings extension.
  const component = getCurrentInstance()
  let connection: InertiaLiveConnection | null = null

  onMounted(() => {
    const page = component?.proxy
    if (page === null || page === undefined) return
    const transport = browserLiveTransport()
    if (transport === null) return
    connection = startInertiaLive({
      getPage: () => page.$page as unknown as InertiaLivePage,
      router: router as unknown as InertiaLiveRouter,
      transport,
    })
  })
  onUnmounted(() => connection?.close())
}
