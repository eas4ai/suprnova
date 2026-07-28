{
  "name": "{project_name}-frontend",
  "private": true,
  "version": "0.1.0",
  "type": "module",
  "scripts": {
    "dev": "vite",
    "build": "vue-tsc --noEmit && vite build",
    "preview": "vite preview",
    "check": "vue-tsc --noEmit"
  },
  "dependencies": {
    "@inertiajs/vue3": "^3.6.1",
    "vue": "^3.5.40"
  },
  "devDependencies": {
    "@tailwindcss/forms": "^0.5.11",
    "@tailwindcss/typography": "^0.5.20",
    "@tailwindcss/vite": "^4.3.3",
    "@types/node": "^24.13.3",
    "@vitejs/plugin-vue": "^6.0.8",
    "tailwindcss": "^4.3.3",
    "typescript": "^6.0.3",
    "vite": "^8.1.5",
    "vue-tsc": "^3.3.8"
  }
}
