<script lang="ts">
  import type { HTMLInputAttributes } from 'svelte/elements';

  interface Props extends Omit<HTMLInputAttributes, 'id'> {
    /** Stable id; generated when omitted so label/input stay associated. */
    id?: string;
    label: string;
    /** Guidance shown under the field. */
    hint?: string;
    /** Error text; also flips `aria-invalid`. */
    error?: string;
    required?: boolean;
  }

  let {
    id = `field-${Math.random().toString(36).slice(2, 9)}`,
    label,
    hint,
    error,
    required = false,
    /*
     * `value` is bindable, and writing to it is what makes `bind:value` work.
     *
     * Without `$bindable()` the compiler accepts the binding and the runtime
     * quietly drops it: the input looks right, keeps its own text, and the
     * parent never hears about a keystroke. That failure is invisible in a
     * screenshot and shows up as "the form submits empty", so it is worth the
     * explicit declaration and the test next to this file.
     */
    value = $bindable(''),
    oninput,
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

  <input
    {id}
    {value}
    oninput={(event) => {
      value = event.currentTarget.value;
      oninput?.(event);
    }}
    aria-invalid={error ? 'true' : undefined}
    aria-describedby={describedBy}
    aria-required={required ? 'true' : undefined}
    {...rest}
  />

  {#if hint}
    <p class="hint" id={hintId}>{hint}</p>
  {/if}
  {#if error}
    <!-- Errors are announced when they appear, not only when read. -->
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

  input {
    font: inherit;
    color: var(--color-text);
    background: var(--color-surface);
    border: var(--border-width) solid var(--color-border-strong);
    border-radius: var(--radius-md);
    padding: var(--space-3);
    width: 100%;
    min-height: 2.75rem;
  }

  input:focus-visible {
    border-color: var(--color-primary);
    outline: 2px solid var(--color-focus);
    outline-offset: 1px;
  }

  input[aria-invalid='true'] {
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
