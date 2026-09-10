<script lang="ts">
  import type { HTMLTextareaAttributes } from 'svelte/elements';

  interface Props extends Omit<HTMLTextareaAttributes, 'id'> {
    /** Stable id; generated when omitted so label/textarea stay associated. */
    id?: string;
    label: string;
    /** Guidance shown under the field. */
    hint?: string;
    /** Error text; also flips `aria-invalid`. */
    error?: string;
    required?: boolean;
    /** Visible rows. Textareas are for prose, so the default is generous. */
    rows?: number;
  }

  let {
    id = `textarea-${Math.random().toString(36).slice(2, 9)}`,
    label,
    hint,
    error,
    required = false,
    rows = 6,
    ...rest
  }: Props = $props();

  let hintId = $derived(`${id}-hint`);
  let errorId = $derived(`${id}-error`);
  let describedBy = $derived(
    [hint ? hintId : null, error ? errorId : null].filter(Boolean).join(' ') || undefined,
  );
</script>

<div class="field">
  <label for={id}>
    {label}
    {#if required}<span class="required" aria-hidden="true">*</span>{/if}
  </label>

  <textarea
    {id}
    {rows}
    aria-invalid={error ? 'true' : undefined}
    aria-describedby={describedBy}
    aria-required={required ? 'true' : undefined}
    {...rest}
  ></textarea>

  {#if hint}
    <p class="hint" id={hintId}>{hint}</p>
  {/if}
  {#if error}
    <p class="error" id={errorId} role="alert">{error}</p>
  {/if}
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
    color: var(--color-text);
  }

  .required {
    color: var(--color-danger);
    margin-left: var(--space-1);
  }

  textarea {
    font: inherit;
    color: var(--color-text);
    background: var(--color-surface);
    border: var(--border-width) solid var(--color-border-strong);
    border-radius: var(--radius-md);
    padding: var(--space-3);
    width: 100%;
    resize: vertical;
  }

  textarea:focus-visible {
    border-color: var(--color-primary);
    outline: 2px solid var(--color-focus);
    outline-offset: 1px;
  }

  textarea[aria-invalid='true'] {
    border-color: var(--color-danger);
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
