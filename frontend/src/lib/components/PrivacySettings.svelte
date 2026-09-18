<script lang="ts">
  import type { PrivacyKeyDescription } from '../api';
  import { describeValue } from '../labels';
  import Button from './Button.svelte';
  import ErrorSummary from './ErrorSummary.svelte';
  import Select from './Select.svelte';

  interface Props {
    /** The keys belonging to the scope being edited. */
    keys: PrivacyKeyDescription[];
    /** Their current values. */
    values: Record<string, string>;
    /** Persist the changed keys. Rejects on failure. */
    onsave: (changes: Record<string, string>) => Promise<void>;
    /** Shown above the controls, e.g. which pseud these apply to. */
    legend: string;
    /** Explains the scope in one sentence. */
    note?: string;
    /** Called after a successful save. */
    onsaved?: () => void;
  }

  let { keys, values, onsave, legend, note, onsaved }: Props = $props();

  let draft = $state<Record<string, string>>({});
  let error = $state<unknown>(null);
  let saving = $state(false);
  let edited = $state(false);

  /**
   * Track the values we seeded the draft from, so a save (which replaces the
   * server's copy) re-seeds rather than looking like a change to overwrite.
   * An unsaved edit wins over a copy that arrives beside it: `edited` is
   * cleared by a save, and not here — this effect reads it as a dependency, so
   * clearing it here would queue another run that seeds after all.
   */
  let seeded = $state('');

  $effect(() => {
    const signature = JSON.stringify(values);
    if (signature !== seeded && !edited) {
      draft = { ...values };
      seeded = signature;
    }
  });

  function markEdited() {
    edited = true;
  }

  /** Only what actually changed is sent. */
  function changed(): Record<string, string> {
    const result: Record<string, string> = {};
    for (const key of keys) {
      const next = draft[key.key] ?? values[key.key] ?? '';
      if (next !== values[key.key]) result[key.key] = next;
    }
    return result;
  }

  let changes = $derived(changed());

  let dirty = $derived(Object.keys(changes).length > 0);

  async function save() {
    if (!dirty) return;
    saving = true;
    error = null;
    try {
      await onsave(changes);
      onsaved?.();
      edited = false;
    } catch (failure) {
      error = failure;
    } finally {
      saving = false;
    }
  }
</script>

<fieldset class="scope">
  <legend>{legend}</legend>
  {#if note}<p class="note">{note}</p>{/if}

  {#if error}
    <ErrorSummary {error} />
  {/if}

  {#each keys as key (key.key)}
    <Select
      id={`privacy-${key.key}`}
      label={key.summary}
      value={draft[key.key] ?? values[key.key] ?? key.values[0]}
      options={key.values.map((value) => ({ value, label: describeValue(value) }))}
      onchange={(event) => {
        markEdited();
        draft = { ...draft, [key.key]: event.currentTarget.value };
      }}
    />
  {/each}

  <div class="actions">
    <Button onclick={save} loading={saving} disabled={!dirty}>
      {dirty ? 'Save changes' : 'Saved'}
    </Button>
    {#if dirty}
      <span class="pending" role="status">Unsaved changes</span>
    {/if}
  </div>
</fieldset>

<style>
  .scope {
    border: var(--border-width) solid var(--color-border);
    border-radius: var(--radius-md);
    padding: var(--space-4);
    margin: 0 0 var(--space-5);
    background: var(--color-surface);
  }

  legend {
    font-family: var(--font-heading);
    font-size: var(--text-lg);
    font-weight: 600;
    padding: 0 var(--space-2);
  }

  .note {
    margin: 0 0 var(--space-4);
    font-size: var(--text-sm);
    color: var(--color-muted);
    max-width: 60ch;
  }

  .actions {
    display: flex;
    align-items: center;
    gap: var(--space-3);
  }

  .pending {
    font-size: var(--text-sm);
    color: var(--color-muted);
  }
</style>
