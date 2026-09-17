<script lang="ts">
  /**
   * Typography controls for the reader (spec §9.2: "typography settings, ...
   * width/line-height controls, distraction-free mode").
   *
   * Two rules, both from the milestone plan:
   *
   *  * **Applied before first paint.** `index.html` reads the stored preference
   *    and sets the same custom properties, so a reader never sees a flash of
   *    the wrong size. This component only *changes* them.
   *  * **Stale writes are conflicts, not overwrites.** The server holds the
   *    version, and a `409` is shown rather than silently retried, so two tabs
   *    cannot clobber each other's settings (spec §3.4 / house rule 2.3).
   *
   * A signed-out reader gets local typography only: the controls still work and
   * are remembered in `localStorage`, they are just not sent anywhere.
   */
  import { fetchTypography, patchTypography, type TypographyView } from '../api';
  import {
    DEFAULT_TYPOGRAPHY,
    READER_THEMES,
    READER_THEME_LABELS,
    applyTypography,
    readTypographyPrefs,
    writeTypographyPrefs,
    type TypographyPrefs,
  } from '../reading';
  import ErrorSummary from './ErrorSummary.svelte';

  interface Props {
    signedIn: boolean;
  }

  let { signedIn }: Props = $props();

  let prefs = $state<TypographyPrefs>(readTypographyPrefs() ?? DEFAULT_TYPOGRAPHY);
  let saving = $state(false);
  let error = $state<unknown>(null);
  let conflict = $state(false);

  // The server is the source of truth for a signed-in reader, and the local
  // copy is what everyone else gets.
  $effect(() => {
    if (!signedIn) return;
    void load();
  });

  $effect(() => {
    applyTypography(prefs, document.documentElement);
    writeTypographyPrefs(prefs);
  });

  async function load() {
    try {
      const view: TypographyView = await fetchTypography();
      // If the server's version is not newer than what we already hold,
      // a concurrent save or a stale in-flight response would clobber
      // local changes made while this request was in flight. Keep the
      // current prefs in that case.
      if (view.version <= prefs.version) return;
      prefs = {
        font_scale: view.font_scale,
        line_height: view.line_height,
        measure: view.measure,
        reader_theme: view.reader_theme,
        distraction_free: view.distraction_free,
        version: view.version,
      };
    } catch {
      // Keep whatever is stored locally; the reader can still read.
    }
  }

  async function save() {
    if (!signedIn) return;
    saving = true;
    error = null;
    conflict = false;
    try {
      await patchTypography({
        expected_version: prefs.version,
        font_scale: prefs.font_scale,
        line_height: prefs.line_height,
        measure: prefs.measure,
        reader_theme: prefs.reader_theme,
        distraction_free: prefs.distraction_free,
      });
      await load();
    } catch (failure) {
      conflict = true;
      error = failure;
    } finally {
      saving = false;
    }
  }
</script>

<fieldset class="typography">
  <legend>Reading</legend>

  <label>
    Text size
    <input type="range" min="0.8" max="1.6" step="0.05" bind:value={prefs.font_scale} />
  </label>

  <label>
    Line height
    <input type="range" min="1.2" max="2.2" step="0.1" bind:value={prefs.line_height} />
  </label>

  <label>
    Line length ({prefs.measure} characters)
    <input type="range" min="40" max="90" step="2" bind:value={prefs.measure} />
  </label>

  <label>
    Reader theme
    <select bind:value={prefs.reader_theme}>
      {#each READER_THEMES as option (option)}
        <option value={option}>{READER_THEME_LABELS[option]}</option>
      {/each}
    </select>
  </label>

  <label class="checkbox">
    <input type="checkbox" bind:checked={prefs.distraction_free} />
    Distraction-free (hide the header and footer while reading)
  </label>

  {#if error}
    {#if conflict}
      <p class="conflict" role="alert">
        These settings changed somewhere else since this page was opened, so this save was
        refused rather than overwriting them.
      </p>
    {/if}
    <ErrorSummary error={error} />
  {/if}

  {#if signedIn}
    <button type="button" onclick={save} disabled={saving}>
      {saving ? 'Saving…' : 'Save settings'}
    </button>
  {:else}
    <p class="note">Sign in to keep these settings across your devices.</p>
  {/if}
</fieldset>

<style>
  .typography {
    border: var(--border-width) solid var(--color-border);
    border-radius: var(--radius-md);
    padding: var(--space-3) var(--space-4);
    font-size: var(--text-sm);
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .typography label {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .typography label.checkbox {
    flex-direction: row;
    align-items: center;
    gap: var(--space-2);
  }

  .conflict {
    color: var(--color-error);
  }

  .note {
    color: var(--color-muted);
    margin: 0;
  }
</style>
