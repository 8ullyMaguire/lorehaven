<script lang="ts">
  import type { Snippet } from 'svelte';
  import type { HTMLButtonAttributes } from 'svelte/elements';

  interface Props extends Omit<HTMLButtonAttributes, 'children'> {
    /** Visual weight. Copper is reserved for emphasis, so `accent` is rare. */
    variant?: 'primary' | 'secondary' | 'quiet' | 'accent' | 'danger';
    size?: 'sm' | 'md';
    /** Shows a busy indicator and blocks interaction. */
    loading?: boolean;
    children?: Snippet;
  }

  let {
    variant = 'primary',
    size = 'md',
    loading = false,
    disabled = false,
    type = 'button',
    children,
    ...rest
  }: Props = $props();
</script>

<button
  {type}
  class="btn {variant} {size}"
  disabled={disabled || loading}
  aria-busy={loading ? 'true' : undefined}
  {...rest}
>
  {#if loading}
    <span class="spinner" aria-hidden="true"></span>
  {/if}
  {@render children?.()}
</button>

<style>
  .btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-2);
    font-family: var(--font-interface);
    font-weight: 600;
    line-height: 1;
    border: var(--border-width) solid transparent;
    border-radius: var(--radius-md);
    cursor: pointer;
    text-decoration: none;
    transition: background-color var(--duration-fast) ease, border-color var(--duration-fast) ease;
  }

  .sm {
    padding: var(--space-2) var(--space-3);
    font-size: var(--text-sm);
  }

  .md {
    padding: var(--space-3) var(--space-5);
    font-size: var(--text-base);
  }

  .primary {
    background: var(--color-primary);
    color: var(--color-primary-contrast);
  }

  .primary:hover:not(:disabled) {
    background: var(--color-primary-hover);
  }

  .secondary {
    background: var(--color-surface);
    border-color: var(--color-border-strong);
    color: var(--color-text);
  }

  .secondary:hover:not(:disabled) {
    border-color: var(--color-primary);
    color: var(--color-primary);
  }

  .quiet {
    background: transparent;
    color: var(--color-primary);
    padding-inline: var(--space-3);
  }

  .quiet:hover:not(:disabled) {
    background: var(--color-accent-soft);
  }

  /* Copper: reserved for milestones and decorative emphasis. */
  .accent {
    background: var(--color-accent);
    color: var(--color-surface);
  }

  .danger {
    background: var(--color-danger);
    color: var(--color-surface);
  }

  .btn:disabled {
    opacity: 0.65;
    cursor: not-allowed;
  }

  .spinner {
    width: 0.9em;
    height: 0.9em;
    border: 2px solid currentColor;
    border-top-color: transparent;
    border-radius: 50%;
    animation: spin 700ms linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .spinner {
      animation-duration: 2s;
    }
  }
</style>
