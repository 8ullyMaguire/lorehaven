<script lang="ts">
  interface Props {
    label: string;
    /** Optional secondary value, e.g. a count. */
    value?: string | number;
    /** `accent` uses copper, reserved for milestones and emphasis. */
    tone?: 'neutral' | 'accent' | 'primary';
    /** Renders as a toggleable filter chip when provided. */
    selected?: boolean;
    onclick?: () => void;
  }

  let { label, value, tone = 'neutral', selected, onclick }: Props = $props();

  let interactive = $derived(typeof onclick === 'function');
</script>

{#if interactive}
  <button
    type="button"
    class="chip {tone}"
    aria-pressed={selected ? 'true' : 'false'}
    onclick={onclick}
  >
    {label}{#if value !== undefined}<span class="value">{value}</span>{/if}
  </button>
{:else}
  <span class="chip {tone}">
    {label}{#if value !== undefined}<span class="value">{value}</span>{/if}
  </span>
{/if}

<style>
  /*
   * The theme asks for restraint: no wall of coloured pills. Chips are quiet
   * by default and only gain colour when selected or explicitly emphasised.
   */
  .chip {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    font-family: var(--font-interface);
    font-size: var(--text-xs);
    font-weight: 600;
    letter-spacing: 0.01em;
    padding: var(--space-1) var(--space-3);
    border: var(--border-width) solid var(--color-border);
    border-radius: var(--radius-pill);
    background: var(--color-surface);
    color: var(--color-muted);
    line-height: 1.6;
  }

  button.chip {
    cursor: pointer;
    font-family: var(--font-interface);
  }

  button.chip:hover {
    border-color: var(--color-border-strong);
    color: var(--color-text);
  }

  .chip[aria-pressed='true'] {
    background: var(--color-primary);
    border-color: var(--color-primary);
    color: var(--color-primary-contrast);
  }

  .primary {
    border-color: var(--color-primary);
    color: var(--color-primary);
  }

  .accent {
    border-color: var(--color-accent);
    color: var(--color-accent);
  }

  .value {
    font-variant-numeric: tabular-nums;
    opacity: 0.8;
  }
</style>
