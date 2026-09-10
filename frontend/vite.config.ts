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
  // Spread, not nested. `svelte()` returns an *array* of plugins (the component
  // plugin, the module plugin and the inspector), so `plugins: [svelte()]`
  // produces a one-element array holding a three-element array. Vite flattens
  // that itself and the build and dev server work either way — but `svelte-check`
  // scans the top level of this array for a plugin named `vite-plugin-svelte*`,
  // does not descend into nested arrays, and therefore reported
  //
  //   Error in vite.config
  //   No Svelte configuration found in vite config. Is
  //   @sveltejs/vite-plugin-svelte configured? (svelte)
  //
  // against all 44 Svelte files in the project. That error is a type-check of
  // nothing: it made `svelte-check` useless as a gate, because 44 of its 65
  // findings were this one line and the real ones were lost among them.
  plugins: [...svelte()],
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
