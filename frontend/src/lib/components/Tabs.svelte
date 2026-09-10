<script lang="ts">
  import type { Snippet } from 'svelte';

  export interface Tab {
    id: string;
    label: string;
  }

  interface Props {
    tabs: Tab[];
    /** The selected tab id. */
    value: string;
    onchange?: (id: string) => void;
    /** Content for the selected tab. */
    children?: Snippet<[string]>;
  }

  let { tabs, value, onchange, children }: Props = $props();

  let tabButtons: HTMLButtonElement[] = $state([]);

  function select(id: string) {
    if (id !== value) onchange?.(id);
  }

  /**
   * Roving tabindex, as the WAI-ARIA tabs pattern requires: only the selected
   * tab is in the tab order, and arrow keys move between tabs.
   */
  function onKeydown(event: KeyboardEvent, index: number) {
    const last = tabs.length - 1;
    let next: number | null = null;

    if (event.key === 'ArrowRight') next = index === last ? 0 : index + 1;
    else if (event.key === 'ArrowLeft') next = index === 0 ? last : index - 1;
    else if (event.key === 'Home') next = 0;
    else if (event.key === 'End') next = last;

    if (next === null) return;
    event.preventDefault();
    const target = tabs[next];
    select(target.id);
    tabButtons[next]?.focus();
  }
</script>

<div class="tabs">
  <div class="tablist" role="tablist">
    {#each tabs as tab, index (tab.id)}
      <button
        bind:this={tabButtons[index]}
        id={`tab-${tab.id}`}
        type="button"
        role="tab"
        class="tab"
        aria-selected={value === tab.id ? 'true' : 'false'}
        aria-controls={`panel-${tab.id}`}
        tabindex={value === tab.id ? 0 : -1}
        onclick={() => select(tab.id)}
        onkeydown={(event) => onKeydown(event, index)}
      >
        {tab.label}
      </button>
    {/each}
  </div>

  <div
    class="panel"
    id={`panel-${value}`}
    role="tabpanel"
    aria-labelledby={`tab-${value}`}
    tabindex="0"
  >
    {@render children?.(value)}
  </div>
</div>

<style>
  .tablist {
    display: flex;
    gap: var(--space-1);
    border-bottom: var(--border-width) solid var(--color-border);
    overflow-x: auto;
  }

  .tab {
    background: transparent;
    border: 0;
    border-bottom: 2px solid transparent;
    color: var(--color-muted);
    font: inherit;
    font-weight: 600;
    padding: var(--space-3) var(--space-4);
    cursor: pointer;
    white-space: nowrap;
  }

  .tab[aria-selected='true'] {
    color: var(--color-text);
    border-bottom-color: var(--color-accent);
  }

  .panel {
    padding-top: var(--space-4);
  }
</style>
