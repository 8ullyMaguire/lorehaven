import { vitePreprocess } from '@sveltejs/vite-plugin-svelte';

/**
 * Svelte options, in the file `vite-plugin-svelte` reads by default.
 *
 * # Why this file exists
 *
 * Not for the build — `vite build` and `vite dev` work without it. It exists
 * because `svelte-check` could not load Svelte options at all without it, and the
 * way that failed made the check useless as a gate.
 *
 * `svelte-check` reads options through `@sveltejs/load-config`, which resolves
 * them by asking Vite for the resolved plugin list and then searching it for a
 * plugin named `vite-plugin-svelte:config`. `@sveltejs/vite-plugin-svelte` 5.1.1
 * does not publish a plugin under that name: its three plugins are
 * `vite-plugin-svelte`, `vite-plugin-svelte-module` and
 * `vite-plugin-svelte-inspector`, and the options live on the first of those as
 * `api.options`. So the search finds nothing, `load-config` reports "No Svelte
 * configuration found in vite config", and `svelte-check` attaches that message
 * to **every `.svelte` file in the project** — 44 errors, all of them the same
 * line, none of them about the code.
 *
 * With this file present, `load-config` falls back to it after the Vite search
 * comes up empty (`loadConfigFromDirectory`: it returns the Svelte config when
 * one is found, and the Vite error only when there is not). The check then runs,
 * and its findings are about the code.
 *
 * The options below are therefore the ones the project was already being built
 * with; the file records them rather than changing them.
 */
export default {
  // TypeScript in `<script lang="ts">` and any future `<style lang="scss">`
  // go through Vite's own transformers (esbuild), which is the documented
  // setup for a Vite + Svelte project and handles more TypeScript than the
  // Svelte compiler's built-in type stripping does. Every script here is
  // `lang="ts"`; every style block is plain CSS.
  preprocess: vitePreprocess(),
};
