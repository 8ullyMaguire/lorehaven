<script lang="ts">
  import type { Snippet } from 'svelte';

  interface Props {
    open?: boolean;
    title: string;
    /** Which edge the panel slides from. */
    side?: 'left' | 'right' | 'bottom';
    onclose?: () => void;
    children?: Snippet;
  }

  let { open = false, title, side = 'right', onclose, children }: Props = $props();

  let panel: HTMLElement | undefined = $state();
  let restoreFocusTo: HTMLElement | null = null;

  $effect(() => {
    if (!open) return;
    restoreFocusTo = (document.activeElement as HTMLElement | null) ?? null;
    const raf = requestAnimationFrame(() => panel?.focus());
    return () => {
      cancelAnimationFrame(raf);
      restoreFocusTo?.focus?.();
    };
  });

  function onKeydown(event: KeyboardEvent) {
    if (event.key === 'Escape') {
      event.stopPropagation();
      onclose?.();
    }
  }
</script>

{#if open}
  <div class="backdrop">
    <button class="scrim" type="button" aria-label="Close panel" onclick={() => onclose?.()}></button>
    <div
      class="panel {side}"
      role="dialog"
      aria-modal="true"
      aria-label={title}
      bind:this={panel}
      tabindex="-1"
      onkeydown={onKeydown}
    >
      <header>
        <h2>{title}</h2>
        <button class="close" type="button" onclick={() => onclose?.()} aria-label="Close">
          <span aria-hidden="true">&times;</span>
        </button>
      </header>
      <div class="body">{@render children?.()}</div>
    </div>
  </div>
{/if}

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 30;
  }

  .scrim {
    position: absolute;
    inset: 0;
    background: var(--color-overlay);
    border: 0;
    cursor: pointer;
  }

  .panel {
    position: absolute;
    background: var(--color-surface-raised);
    border: var(--border-width) solid var(--color-border);
    box-shadow: var(--shadow-lg);
    display: flex;
    flex-direction: column;
    overflow: auto;
    padding: var(--space-5);
  }

  .right {
    top: 0;
    right: 0;
    bottom: 0;
    width: min(24rem, 90vw);
  }

  .left {
    top: 0;
    left: 0;
    bottom: 0;
    width: min(24rem, 90vw);
  }

  .bottom {
    left: 0;
    right: 0;
    bottom: 0;
    max-height: 85vh;
    border-radius: var(--radius-lg) var(--radius-lg) 0 0;
  }

  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
  }

  header h2 {
    margin: 0;
  }

  .close {
    background: transparent;
    border: 0;
    font-size: var(--text-xl);
    color: var(--color-muted);
    cursor: pointer;
  }

  .body {
    margin-top: var(--space-4);
  }
</style>
