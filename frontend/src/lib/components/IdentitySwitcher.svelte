<script lang="ts">
  export interface PseudSummary {
    id: string;
    handle: string;
    displayName: string;
  }

  interface Props {
    pseuds: PseudSummary[];
    activeId: string;
    onswitch?: (id: string) => void;
  }

  let { pseuds, activeId, onswitch }: Props = $props();

  let active = $derived(pseuds.find((p) => p.id === activeId) ?? pseuds[0]);
</script>

<!--
  Identity switcher (Milestone 1). Pseuds are separate public faces of one
  account (spec §7): the switcher says who you are posting as, and deliberately
  says nothing about what else the account holds.
-->
<div class="switcher">
  <span class="label">Writing as</span>
  <div class="options" role="group" aria-label="Active pseud">
    {#each pseuds as pseud (pseud.id)}
      <button
        type="button"
        class="pseud"
        aria-pressed={pseud.id === active?.id ? 'true' : 'false'}
        onclick={() => onswitch?.(pseud.id)}
      >
        <span class="handle">@{pseud.handle}</span>
        <span class="name">{pseud.displayName}</span>
      </button>
    {/each}
  </div>
</div>

<style>
  .switcher {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    flex-wrap: wrap;
  }

  .label {
    font-size: var(--text-sm);
    color: var(--color-muted);
  }

  .options {
    display: flex;
    gap: var(--space-2);
    flex-wrap: wrap;
  }

  .pseud {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 0.1rem;
    background: var(--color-surface);
    border: var(--border-width) solid var(--color-border);
    border-radius: var(--radius-md);
    padding: var(--space-2) var(--space-3);
    cursor: pointer;
    font: inherit;
    color: var(--color-text);
    text-align: left;
  }

  .pseud[aria-pressed='true'] {
    border-color: var(--color-accent);
    box-shadow: inset 0 0 0 1px var(--color-accent);
  }

  .handle {
    font-weight: 600;
  }

  .name {
    font-size: var(--text-xs);
    color: var(--color-muted);
  }
</style>
