{
  "name": "{project_name}-frontend",
  "private": true,
  "version": "0.1.0",
  "type": "module",
  "scripts": {
    "dev": "vite",
    "build": "tsc && vite build",
    "build:ssr": "vite build --ssr src/ssr.tsx",
    "preview": "vite preview"
  },
  "dependencies": {
    "@fluent/bundle": "^0.19.1",
    "@inertiajs/react": "^3.6.1",
    "react": "^19.2.8",
    "react-dom": "^19.2.8"
  },
  "devDependencies": {
    "@tailwindcss/forms": "^0.5.11",
    "@tailwindcss/typography": "^0.5.20",
    "@tailwindcss/vite": "^4.3.3",
    "@types/node": "^24.13.3",
    "@types/react": "^19.2.17",
    "@types/react-dom": "^19.2.3",
    "@vitejs/plugin-react": "^6.0.4",
    "tailwindcss": "^4.3.3",
    "typescript": "^6.0.3",
    "vite": "^8.1.5"
  }
}
