<script lang="ts">
  import type { ContentSettings } from '../api';
  import { RATINGS, describeRating } from '../labels';
  import Button from './Button.svelte';
  import ErrorSummary from './ErrorSummary.svelte';
  import Select from './Select.svelte';
  import Textarea from './Textarea.svelte';

  interface Props {
    settings: ContentSettings;
    /** Persist the change. Rejects on failure. */
    onsave: (patch: {
      expected_version: number;
      max_rating?: string;
      excluded_warnings?: string[];
    }) => Promise<void>;
    /** Re-read after a conflict. */
    onreload: () => void;
  }

  let { settings, onsave, onreload }: Props = $props();

  // Seeded from the server's copy by the effect below rather than in the
  // initialiser: reading a prop into `$state` captures it once and silently
  // stops tracking, which is exactly the bug this form must not have.
  let rating = $state('');
  let warnings = $state('');
  let error = $state<unknown>(null);
  let saving = $state(false);
  let saved = $state(false);
  let edited = $state(false);

  // Re-seed when the server's copy changes (after a save, or a reload).
  let seeded = $state('');
  $effect(() => {
    const signature = `${settings.version}:${settings.max_rating}`;
    if (signature !== seeded && !edited) {
      rating = settings.max_rating;
      warnings = settings.excluded_warnings.join('\n');
      seeded = signature;
      error = null;
    }
    edited = false;
  });

  function markEdited() {
    edited = true;
  }

  function parsedWarnings(): string[] {
    return warnings
      .split('\n')
      .map((line) => line.trim())
      .filter((line) => line.length > 0);
  }

  // `seeded` guards the first frame, before the effect has run: an unseeded
  // form would otherwise look as though the reader had changed something.
  let dirty = $derived(
    seeded !== '' &&
      (rating !== settings.max_rating ||
        parsedWarnings().join('\n') !== settings.excluded_warnings.join('\n')),
  );

  async function save() {
    saving = true;
    error = null;
    saved = false;
    try {
      await onsave({
        expected_version: settings.version,
        max_rating: rating,
        excluded_warnings: parsedWarnings(),
      });
      saved = true;
      edited = false;
    } catch (failure) {
      error = failure;
    } finally {
      saving = false;
    }
  }

  let ratingOptions = RATINGS.map((value) => ({ value, label: describeRating(value) }));

  /** Position on the rating ladder, or -1 when the server sent something new. */
  function rank(value: string): number {
    return RATINGS.indexOf(value as (typeof RATINGS)[number]);
  }

  /**
   * Whether the chosen rating is above what the instance will ever show this
   * account. The server decides this and reports it; the interface only
   * explains the result rather than pretending the choice took effect.
   */
  let capped = $derived(rank(settings.effective_max_rating) >= 0 && rank(rating) > rank(settings.effective_max_rating));
</script>

{#if error}
  <ErrorSummary {error} onretry={onreload} />
{/if}

<Select
  id="content-max-rating"
  label="Show me works rated up to"
  options={ratingOptions}
  value={rating}
  onchange={(event) => { markEdited(); rating = event.currentTarget.value; }}
  />

<p class="ceiling">
  This instance will show this account works rated up to
  <strong>{describeRating(settings.policy_ceiling)}</strong>.
  {#if settings.effective_max_rating !== settings.max_rating}
    Your saved preference of {describeRating(settings.max_rating)} is above that, so
    {describeRating(settings.effective_max_rating)} is what is actually in force.
  {:else}
    What you choose below that is what takes effect.
  {/if}
</p>

{#if capped}
  <p class="warning" role="status">
    Saving a preference above the instance ceiling is allowed and remembered, but it will
    not change what you are shown.
  </p>
{/if}

<Textarea
  id="content-warnings"
  label="Warnings I never want to see"
  hint="One tag per line, for example: major character death. Matching is exact and case-insensitive."
  rows={5}
  value={warnings}
  oninput={(event) => { markEdited(); warnings = event.currentTarget.value }}
  />

<div class="actions">
  <Button onclick={save} loading={saving} disabled={!dirty && !error}>Save changes</Button>
  {#if saved && !dirty}
    <span class="ok" role="status">Saved</span>
  {/if}
</div>

<style>
  .ceiling {
    margin: calc(-1 * var(--space-2)) 0 var(--space-4);
    font-size: var(--text-sm);
    color: var(--color-muted);
    max-width: 60ch;
  }

  .warning {
    margin: 0 0 var(--space-4);
    padding: var(--space-3);
    border-left: 3px solid var(--color-accent);
    background: var(--color-accent-soft);
    border-radius: var(--radius-sm);
    font-size: var(--text-sm);
  }

  .actions {
    display: flex;
    align-items: center;
    gap: var(--space-3);
  }

  .ok {
    font-size: var(--text-sm);
    color: var(--color-success);
    font-weight: 600;
  }
</style>
