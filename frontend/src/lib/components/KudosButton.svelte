<script lang="ts">
  /**
   * Kudos button for a work page (spec §9.4, §35).
   *
   * One per signed-in account; visitors see a note to sign in. The server
   * owns the count; this component just reflects the current state.
   */
  import { toggleKudos } from '../api';
  import { session } from '../session.svelte.ts';

  interface Props {
    workId: string;
  }

  let { workId }: Props = $props();

  let kudoed = $state(false);
  let busy = $state(false);
  let error = $state<unknown>(null);

  async function toggle() {
    if (!session.isSignedIn || busy) return;
    busy = true;
    error = null;
    try {
      const res = await toggleKudos(workId);
      kudoed = res.kudoed;
    } catch (failure) {
      error = failure;
    } finally {
      busy = false;
    }
  }
</script>

<div class="kudos">
  {#if session.isSignedIn}
    <button
      type="button"
      class="kudos-button"
      class:kudoed
      onclick={toggle}
      disabled={busy}
      aria-pressed={kudoed}
    >
      {kudoed ? 'Kudoed ♥' : 'Kudos'}
    </button>
  {:else}
    <a href="/sign-in" class="kudos-login">Sign in to leave kudos</a>
  {/if}
</div>

<style>
  .kudos-button {
    padding: var(--space-2) var(--space-3);
    border: var(--border-width) solid var(--color-border);
    border-radius: var(--radius-sm);
    background: var(--color-surface);
    cursor: pointer;
    font-size: var(--text-sm);
  }

  .kudos-button.kudoed {
    border-color: var(--color-accent);
    color: var(--color-accent);
  }

  .kudos-button:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .kudos-login {
    font-size: var(--text-sm);
    color: var(--color-muted);
  }
</style>
