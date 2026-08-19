import { defineConfig } from 'vite'
import { svelte } from '@sveltejs/vite-plugin-svelte'
import tailwindcss from '@tailwindcss/vite'

export default defineConfig(({ isSsrBuild }) => ({
  plugins: [tailwindcss(), svelte()],
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
        // `public/assets` alongside the client bundle — `suprnova
        // ssr:start` looks for `frontend/bootstrap/ssr/ssr.js` by
        // default (see `suprnova-cli/src/commands/ssr_start.rs`).
        outDir: 'bootstrap/ssr',
        rollupOptions: {
          output: {
            // Pin the filename — Vite's default naming for an SSR entry
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
