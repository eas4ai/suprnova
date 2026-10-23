import { defineConfig } from 'vite'
import inertia from '@inertiajs/vite'
import { svelte } from '@sveltejs/vite-plugin-svelte'
import tailwindcss from '@tailwindcss/vite'

export default defineConfig({
  // `inertia()` turns `pages: './pages'` in `src/main.ts` into the page
  // resolver, one lazily loaded chunk per page
  // (`inertia-3.8.0/packages/vite/src/pagesTransform.ts`). This app has no
  // SSR entry, so the plugin's SSR side finds nothing to serve.
  plugins: [inertia(), tailwindcss(), svelte()],
  server: {
    port: 5173,
    strictPort: true,
    cors: true,
  },
  build: {
    outDir: '../public/assets',
    manifest: true,
    rollupOptions: {
      input: 'src/main.ts',
    },
  },
})
