import { defineConfig } from 'vite'
import inertia from '@inertiajs/vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'

export default defineConfig(({ isSsrBuild }) => ({
  // Relative, so the built entry loads its chunks and assets from where the
  // server's tags put it: the same build runs at `/` and behind a proxy that
  // serves the application under a path prefix such as `/billing`.
  base: './',
  plugins: [
    // The Inertia plugin turns `pages: './pages'` in both entries into a
    // resolver over `src/pages/**/*.tsx`, and turns `src/ssr.tsx` into the
    // SSR bundle's render function: in development the dev server answers
    // `/__inertia_ssr` with it, and in a production build it is wrapped in
    // `createServer` from `@inertiajs/react/server`. Naming the entry here
    // keeps the plugin and `npm run build:ssr` on the same file
    // (`packages/vite/src/index.ts`, `ssr.ts` in Inertia 3.8.0).
    inertia({ ssr: 'src/ssr.tsx' }),
    tailwindcss(),
    react(),
  ],
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
        // `vite build --ssr src/ssr.tsx` lands here, not in
        // `public/assets` alongside the client bundle - `suprnova
        // ssr:start` looks for `frontend/bootstrap/ssr/ssr.js` by
        // default (see `suprnova-cli/src/commands/ssr_start.rs`). The
        // Inertia plugin adds the entry as the build's input and a source
        // map beside the bundle; it leaves the output directory and the
        // file name to this config.
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
          input: 'src/main.tsx',
        },
      },
}))
