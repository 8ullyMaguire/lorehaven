<script lang="ts">
  import type { Snippet } from 'svelte';

  interface Props {
    open?: boolean;
    /** Accessible name for the dialog. */
    title: string;
    /** Optional supporting line under the title. */
    description?: string;
    /** Called for Escape, backdrop click and the close button. */
    onclose?: () => void;
    children?: Snippet;
    footer?: Snippet;
  }

  let { open = false, title, description, onclose, children, footer }: Props = $props();

  let panel: HTMLDivElement | undefined = $state();
  let restoreFocusTo: HTMLElement | null = null;

  const FOCUSABLE = [
    'a[href]',
    'button:not([disabled])',
    'input:not([disabled])',
    'select:not([disabled])',
    'textarea:not([disabled])',
    '[tabindex]:not([tabindex="-1"])',
  ].join(',');

  /**
   * Focus management (Milestone 1 acceptance): focus is trapped inside while
   * open, moved in when it opens, and restored to the invoking control when it
   * closes — including when the close is caused by unmounting.
   */
  $effect(() => {
    if (!open) return;

    restoreFocusTo = (document.activeElement as HTMLElement | null) ?? null;

    const raf = requestAnimationFrame(() => {
      const first = panel?.querySelector<HTMLElement>(FOCUSABLE);
      (first ?? panel)?.focus();
    });

    return () => {
      cancelAnimationFrame(raf);
      restoreFocusTo?.focus?.();
    };
  });

  function focusables(): HTMLElement[] {
    if (!panel) return [];
    return Array.from(panel.querySelectorAll<HTMLElement>(FOCUSABLE)).filter(
      (element) => element.offsetParent !== null || element === document.activeElement,
    );
  }

  function onKeydown(event: KeyboardEvent) {
    if (event.key === 'Escape') {
      event.stopPropagation();
      onclose?.();
      return;
    }
    if (event.key !== 'Tab') return;

    const items = focusables();
    if (items.length === 0) {
      event.preventDefault();
      panel?.focus();
      return;
    }
    const first = items[0];
    const last = items[items.length - 1];
    const active = document.activeElement as HTMLElement | null;

    if (event.shiftKey && (active === first || active === panel)) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && active === last) {
      event.preventDefault();
      first.focus();
    }
  }
</script>

{#if open}
  <div class="backdrop">
    <!-- Backdrop dismissal: a convenience, never the only way to close. -->
    <button class="scrim" type="button" aria-label="Close dialog" onclick={() => onclose?.()}></button>

    <div
      class="panel"
      role="dialog"
      aria-modal="true"
      aria-labelledby="dialog-title"
      aria-describedby={description ? 'dialog-description' : undefined}
      bind:this={panel}
      tabindex="-1"
      onkeydown={onKeydown}
    >
      <header>
        <h2 id="dialog-title">{title}</h2>
        <button class="close" type="button" onclick={() => onclose?.()} aria-label="Close">
          <span aria-hidden="true">&times;</span>
        </button>
      </header>

      {#if description}
        <p class="description" id="dialog-description">{description}</p>
      {/if}

      <div class="body">{@render children?.()}</div>

      {#if footer}
        <footer>{@render footer()}</footer>
      {/if}
    </div>
  </div>
{/if}

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 40;
    display: grid;
    place-items: center;
    padding: var(--space-4);
  }

  .scrim {
    position: absolute;
    inset: 0;
    background: var(--color-overlay);
    border: 0;
    padding: 0;
    cursor: pointer;
  }

  .panel {
    position: relative;
    width: min(34rem, 100%);
    max-height: 85vh;
    overflow: auto;
    background: var(--color-surface-raised);
    color: var(--color-text);
    border: var(--border-width) solid var(--color-border);
    border-radius: var(--radius-lg);
    box-shadow: var(--shadow-lg);
    padding: var(--space-5);
  }

  header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--space-4);
  }

  header h2 {
    margin: 0;
  }

  .close {
    background: transparent;
    border: 0;
    color: var(--color-muted);
    font-size: var(--text-xl);
    line-height: 1;
    cursor: pointer;
    padding: var(--space-1) var(--space-2);
    border-radius: var(--radius-sm);
  }

  .close:hover {
    color: var(--color-text);
  }

  .description {
    color: var(--color-muted);
    margin-top: var(--space-2);
  }

  .body {
    margin-top: var(--space-4);
  }

  footer {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-3);
    margin-top: var(--space-5);
  }
</style>
