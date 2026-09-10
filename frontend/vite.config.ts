import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';

// The production build is a static bundle: spec §2.1 notes the production
// application does not need a Node.js server, and `crates/app/build.rs` embeds
// whatever lands in `dist/` into the Rust binary.
//
// During development the API is proxied to the local server, so the browser
// sees one origin and the cookie/CSRF design is exercised exactly as it will
// be in production.
export default defineConfig({
  plugins: [svelte()],
  // SvelteKit calls this directory `static`; Vite defaults to `public`. The
  // name is kept because the rest of the project's documentation uses it.
  publicDir: 'static',
  build: {
    outDir: 'dist',
    emptyOutDir: true,
    target: 'es2022',
    sourcemap: true,
  },
  server: {
    port: 5173,
    strictPort: true,
    proxy: {
      '/api': 'http://127.0.0.1:8080',
      '/health': 'http://127.0.0.1:8080',
    },
  },
});
