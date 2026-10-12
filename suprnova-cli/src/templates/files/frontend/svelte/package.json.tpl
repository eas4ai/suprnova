{
  "name": "{project_name}-frontend",
  "private": true,
  "version": "0.1.0",
  "type": "module",
  "scripts": {
    "dev": "vite",
    "build": "svelte-check --tsconfig ./tsconfig.json && vite build",
    "build:ssr": "vite build --ssr src/ssr.ts",
    "preview": "vite preview",
    "check": "svelte-check --tsconfig ./tsconfig.json"
  },
  "dependencies": {
    "@fluent/bundle": "^0.19.1",
    "@inertiajs/core": "^3.8.0",
    "@inertiajs/svelte": "^3.8.0",
    "svelte": "^5.56.8"
  },
  "devDependencies": {
    "@inertiajs/vite": "^3.8.0",
    "@sveltejs/vite-plugin-svelte": "^7.2.0",
    "@tailwindcss/forms": "^0.5.11",
    "@tailwindcss/typography": "^0.5.20",
    "@tailwindcss/vite": "^4.3.3",
    "@tsconfig/svelte": "^5.0.8",
    "@types/node": "^24.13.3",
    "svelte-check": "^4.7.4",
    "tailwindcss": "^4.3.3",
    "typescript": "^6.0.3",
    "vite": "^8.1.5"
  }
}
