<script lang="ts">
  import type { HTMLSelectAttributes } from 'svelte/elements';

  export interface SelectOption {
    value: string;
    label: string;
    disabled?: boolean;
  }

  interface Props extends Omit<HTMLSelectAttributes, 'id' | 'children'> {
    id?: string;
    label: string;
    options: SelectOption[];
    hint?: string;
    error?: string;
  }

  let {
    id = `select-${Math.random().toString(36).slice(2, 9)}`,
    label,
    options,
    hint,
    error,
    /** Bindable; see the note in `TextField.svelte`. */
    value = $bindable(''),
    onchange,
    ...rest
  }: Props = $props();

  let hintId = $derived(`${id}-hint`);
  let errorId = $derived(`${id}-error`);
  let describedBy = $derived(
    [hint ? hintId : null, error ? errorId : null].filter(Boolean).join(' ') || undefined,
  );
</script>

<div class="field">
  <label for={id}>{label}</label>
  <select
    {id}
    {value}
    onchange={(event) => {
      value = event.currentTarget.value;
      onchange?.(event);
    }}
    aria-invalid={error ? 'true' : undefined}
    aria-describedby={describedBy}
    {...rest}
  >
    {#each options as option (option.value)}
      <option value={option.value} disabled={option.disabled}>{option.label}</option>
    {/each}
  </select>
  {#if hint}<p class="hint" id={hintId}>{hint}</p>{/if}
  {#if error}<p class="error" id={errorId} role="alert">{error}</p>{/if}
</div>

<style>
  .field {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin-bottom: var(--space-4);
  }

  label {
    font-size: var(--text-sm);
    font-weight: 600;
  }

  select {
    font: inherit;
    color: var(--color-text);
    background: var(--color-surface);
    border: var(--border-width) solid var(--color-border-strong);
    border-radius: var(--radius-md);
    padding: var(--space-3);
    min-height: 2.75rem;
    width: 100%;
  }

  .hint {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--color-muted);
  }

  .error {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--color-danger);
    font-weight: 600;
  }
</style>
