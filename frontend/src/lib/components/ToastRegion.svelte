<script lang="ts">
  export interface Toast {
    id: string;
    message: string;
    tone?: 'info' | 'success' | 'danger';
  }

  interface Props {
    toasts: Toast[];
    ondismiss?: (id: string) => void;
  }

  let { toasts, ondismiss }: Props = $props();
</script>

<!--
  A polite live region: screen readers announce toasts when there is a pause,
  which is right for confirmations. `aria-atomic` keeps each message whole.
-->
<div class="region" role="status" aria-live="polite" aria-atomic="false">
  {#each toasts as toast (toast.id)}
    <div class="toast {toast.tone ?? 'info'}">
      <p>{toast.message}</p>
      <button type="button" onclick={() => ondismiss?.(toast.id)} aria-label="Dismiss notification">
        <span aria-hidden="true">&times;</span>
      </button>
    </div>
  {/each}
</div>

<style>
  .region {
    position: fixed;
    right: var(--space-4);
    bottom: var(--space-4);
    z-index: 50;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    max-width: min(24rem, calc(100vw - var(--space-8)));
  }

  /* An empty live region must not occupy pointer space. */
  .region:empty {
    display: none;
  }

  .toast {
    display: flex;
    align-items: flex-start;
    gap: var(--space-3);
    background: var(--color-surface-raised);
    border: var(--border-width) solid var(--color-border);
    border-left: 3px solid var(--color-muted);
    border-radius: var(--radius-md);
    box-shadow: var(--shadow-md);
    padding: var(--space-3) var(--space-4);
  }

  .toast.success {
    border-left-color: var(--color-success);
  }

  .toast.danger {
    border-left-color: var(--color-danger);
  }

  p {
    margin: 0;
    font-size: var(--text-sm);
  }

  button {
    background: transparent;
    border: 0;
    color: var(--color-muted);
    cursor: pointer;
    font-size: var(--text-lg);
    line-height: 1;
  }
</style>
