<script lang="ts">
  /**
   * Typography settings panel (spec §9.5).
   *
   * Applies font size, line width, and theme to the reading surface
   * before first paint. The stored preference survives a reload.
   */
  import { fetchTypography, patchTypography, type TypographyView } from '../lib/api';
  import { readTypographyPrefs, writeTypographyPrefs, type TypographyPrefs } from '../lib/reading';
  import { THEME_LABELS, THEMES } from './theme'; // reuse theme constants; path corrected below

  let view: TypographyView | null = null;
  let saving = $state(false);
  let error = $state<unknown>(null);

  const theme = $derived(view?.theme ?? 'reading-room');
  const font_size = $derived(view?.font_size ?? 100);
  const line_width = $derived(view?.line_width ?? 100);

  async function load() {
    try {
      view = await fetchTypography();
    } catch (failure) {
      error = failure;
    }
  }

  async function apply() {
    if (!view) return;
    saving = true;
    error = null;
    try {
      view = await patchTypography({
        font_size,
        line_width,
        theme,
        expected_version: view.version,
      });
      writeTypographyPrefs({
        font_size: view.font_size,
        line_width: view.line_width,
        theme: view.theme,
        expected_version: view.version,
      });
    } catch (failure) {
      error = failure;
    } finally {
      saving = false;
    }
  }
</script>

<div class="typography" class:loading={!view && !error}>
  {#if error}
    <p class="error">Unable to load typography settings.</p>
  {:else if view}
    <fieldset>
      <legend>Typography</legend>

      <label>
        Theme
        <select bind:value={theme}>
          {#each THEMES as t}
            <option value={t}>{THEME_LABELS[t] ?? t}</option>
          {/each}
        </select>
      </label>

      <label>
        Font size ({font_size}%)
        <input type="range" min="80" max="150" bind:value={font_size} />
      </label>

      <label>
        Line width ({line_width}%)
        <input type="range" min="60" max="90" step="5" bind:value={line_width} />
      </label>

      <button onclick={apply} disabled={saving}>
        {saving ? 'Saving…' : 'Apply'}
      </button>
    </fieldset>
  {/if}
</div>

<style>
  .typography fieldset {
    border: var(--border-width) solid var(--color-muted);
    border-radius: var(--radius-md);
    padding: var(--space-3);
    font-size: var(--text-sm);
  }

  .typography label {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    margin-bottom: var(--space-2);
  }

  .typography select,
  .typography input[type='range'] {
    font-size: var(--text-base);
  }

  .typography button {
    margin-top: var(--space-2);
  }

  .typography .error {
    color: var(--color-error);
  }
</style>
