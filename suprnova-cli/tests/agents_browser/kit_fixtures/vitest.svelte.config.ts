import { svelte } from '@sveltejs/vite-plugin-svelte'
import { defineConfig } from 'vitest/config'

export default defineConfig({
  plugins: [svelte()],
  // The browser build of Svelte, which has `mount`.
  resolve: { conditions: ['browser'] },
  test: { environment: 'jsdom', include: ['agents-core.test.ts', 'agents.test.ts'] },
})
