import { defineConfig } from 'vite'
import inertia from '@inertiajs/vite'
import { svelte } from '@sveltejs/vite-plugin-svelte'
import tailwindcss from '@tailwindcss/vite'

export default defineConfig(({ isSsrBuild }) => ({
  // Relative, so the built entry loads its chunks and assets from where the
  // server's tags put it: the same build runs at `/` and behind a proxy that
  // serves the application under a path prefix such as `/billing`.
  base: './',
  plugins: [
    tailwindcss(),
    // The Inertia plugin (@inertiajs/vite 3.8.0, packages/vite/src/index.ts)
    // turns `pages: './pages'` in both entries into the page resolver, and
    // wraps `src/ssr.ts` into the SSR server on an SSR build. Under
    // `npm run dev` it also serves that entry at `/__inertia_ssr`, the
    // endpoint `suprnova serve`'s hot file sends a first visit to. Naming
    // the entry here keeps that endpoint and `npm run build:ssr` on the
    // same file. The plugin would also write source maps for the SSR
    // bundle; `suprnova ssr:start` runs Node without
    // `--enable-source-maps`, so nothing reads them, and the plugin's own
    // rewrite of the entry leaves that entry's map wrong, which the build
    // reports as a warning.
    inertia({ ssr: { entry: 'src/ssr.ts', sourcemap: false } }),
    svelte(),
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
        // `vite build --ssr src/ssr.ts` lands here, not in
        // `public/assets` alongside the client bundle - `suprnova
        // ssr:start` looks for `frontend/bootstrap/ssr/ssr.js` first
        // (`suprnova::SsrConfig`'s conventional bundle paths). The
        // plugin sets the SSR build's input to the entry above and turns
        // on its source map; it sets no output path, so this one stands.
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
