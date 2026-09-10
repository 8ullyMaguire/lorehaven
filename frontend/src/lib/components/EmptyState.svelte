<script lang="ts">
  import type { Snippet } from 'svelte';

  interface Props {
    title: string;
    description?: string;
    /** Optional action area, e.g. a button or a link. */
    action?: Snippet;
  }

  let { title, description, action }: Props = $props();
</script>

<div class="empty">
  <!--
    The theme's empty-state motif: a subtle arch, the same shape as the logo's
    negative space. Decorative only, so it is hidden from assistive tech.
  -->
  <svg class="arch" viewBox="0 0 48 48" aria-hidden="true" focusable="false">
    <path
      d="M8 42V20a16 16 0 0 1 32 0v22"
      fill="none"
      stroke="currentColor"
      stroke-width="1.5"
      stroke-linecap="round"
    />
    <path d="M18 42V22a6 6 0 0 1 12 0v20" fill="none" stroke="currentColor" stroke-width="1.5" />
  </svg>

  <h3>{title}</h3>
  {#if description}
    <p>{description}</p>
  {/if}
  {#if action}
    <div class="action">{@render action()}</div>
  {/if}
</div>

<style>
  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    text-align: center;
    gap: var(--space-2);
    padding: var(--space-7) var(--space-4);
    background: var(--color-surface);
    border: var(--border-width) dashed var(--color-border-strong);
    border-radius: var(--radius-lg);
  }

  .arch {
    width: 3.5rem;
    height: 3.5rem;
    color: var(--color-border-strong);
    margin-bottom: var(--space-2);
  }

  h3 {
    margin: 0;
  }

  p {
    margin: 0;
    color: var(--color-muted);
    max-width: 32rem;
  }

  .action {
    margin-top: var(--space-3);
  }
</style>
