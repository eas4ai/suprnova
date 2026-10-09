import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
import tailwindcss from '@tailwindcss/vite'
import inertia from '@inertiajs/vite'

export default defineConfig(({ isSsrBuild }) => ({
  // Relative, so the built entry loads its chunks and assets from where the
  // server's tags put it: the same build runs at `/` and behind a proxy that
  // serves the application under a path prefix such as `/billing`.
  base: './',
  // `inertia()` (Inertia 3.8.0, `packages/vite/src/index.ts`) does three
  // things here. It turns `pages: './pages'` in both entries into a
  // resolver over `src/pages/**/*.vue`. In development it serves the SSR
  // entry at `/__inertia_ssr` on this dev server, where the framework
  // sends a first visit while the hot file names the dev server. And on
  // `vite build --ssr` it wraps the SSR entry's top-level
  // `createInertiaApp(..)` call with `createServer` (`ssrTransform.ts`,
  // `frameworks/vue.ts`), keeps `src/ssr.ts` as the input (the first of
  // its entry candidates this project has, `ssr.ts`), and writes a source
  // map beside the server bundle.
  plugins: [inertia(), tailwindcss(), vue()],
  server: {
    // `suprnova serve` sets VITE_PORT to the port it resolved (the
    // distinctive 5765 default, or a scanned free port). Falling back to
    // 5765 keeps a bare `npm run dev` off the squatted 5173.
    port: Number(process.env.VITE_PORT) || 5765,
    strictPort: true,
    cors: true,
  },
  build: isSsrBuild
    ? {
        // `vite build --ssr src/ssr.ts` lands here, not in
        // `public/assets` alongside the client bundle - `suprnova
        // ssr:start` looks for `frontend/bootstrap/ssr/ssr.js` first
        // (`CONVENTIONAL_BUNDLE_PATHS` in the framework's
        // `inertia/ssr.rs`).
        outDir: 'bootstrap/ssr',
        rollupOptions: {
          output: {
            // Pin the filename - Vite's default naming for an SSR entry
            // is not a contract `ssr:start`'s bundle-path default can
            // rely on.
            entryFileNames: 'ssr.js',
          },
        },
      }
    : {
        outDir: '../public/assets',
        manifest: true,
        rollupOptions: {
          input: 'src/main.ts',
        },
      },
}))
