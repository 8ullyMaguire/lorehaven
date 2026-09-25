<script lang="ts">
  /**
   * Moderation panel for forum topics (spec §35.5).
   *
   * Provides controls for: sanctions, slow mode, federation scope,
   * featuring posts. Only visible to moderators (trust level >= moderator).
   */
  import {
    applySanction,
    checkSanction,
    setFederationScope,
    setSlowMode,
    type Sanction,
  } from '../api';

  let { topicId, isModerator }: { topicId: string; isModerator: boolean } =
    $props();

  let slowModeSeconds = $state(0);
  let federationScope = $state<'public' | 'local' | 'unlisted'>('public');
  let sanctionLevel = $state<'warning' | 'mute' | 'forum_ban' | 'site_ban'>(
    'warning',
  );
  let sanctionReason = $state('');
  let sanctionTarget = $state('');
  let activeSanction = $state<Sanction | null>(null);
  let error = $state<unknown>(null);
  let saving = $state(false);

  async function loadSanction() {
    if (!sanctionTarget) return;
    try {
      activeSanction = await checkSanction(sanctionTarget, topicId);
    } catch {
      activeSanction = null;
    }
  }

  async function saveSlowMode() {
    saving = true;
    error = null;
    try {
      await setSlowMode(topicId, slowModeSeconds);
    } catch (failure) {
      error = failure;
    } finally {
      saving = false;
    }
  }

  async function saveFederationScope() {
    saving = true;
    error = null;
    try {
      await setFederationScope(topicId, federationScope);
    } catch (failure) {
      error = failure;
    } finally {
      saving = false;
    }
  }

  async function applySanctionAction() {
    if (!sanctionTarget || !sanctionReason) return;
    saving = true;
    error = null;
    try {
      await applySanction({
        account: sanctionTarget,
        level: sanctionLevel,
        reason: sanctionReason,
        category_id: topicId,
      });
      sanctionReason = '';
      await loadSanction();
    } catch (failure) {
      error = failure;
    } finally {
      saving = false;
    }
  }
</script>

{#if isModerator}
  <div class="moderation-panel">
    <h3>Moderation</h3>

    <!-- Slow mode -->
    <div class="control">
      <label for="slow-mode">Slow mode (seconds)</label>
      <input
        id="slow-mode"
        type="number"
        bind:value={slowModeSeconds}
        min="0"
      />
      <button onclick={saveSlowMode} disabled={saving}>
        {saving ? 'Saving…' : 'Set slow mode'}
      </button>
    </div>

    <!-- Federation scope -->
    <div class="control">
      <label for="federation-scope">Federation scope</label>
      <select id="federation-scope" bind:value={federationScope}>
        <option value="public">Public</option>
        <option value="local">Local only</option>
        <option value="unlisted">Unlisted</option>
      </select>
      <button onclick={saveFederationScope} disabled={saving}>
        Set scope
      </button>
    </div>

    <!-- Sanctions -->
    <div class="control">
      <h4>Sanction user</h4>
      <input type="text" bind:value={sanctionTarget} placeholder="Account" />
      <select bind:value={sanctionLevel}>
        <option value="warning">Warning</option>
        <option value="mute">Mute</option>
        <option value="forum_ban">Forum ban</option>
        <option value="site_ban">Site ban</option>
      </select>
      <input
        type="text"
        bind:value={sanctionReason}
        placeholder="Reason"
      />
      <button onclick={applySanctionAction} disabled={saving || !sanctionTarget}>
        Apply
      </button>
      {#if activeSanction}
        <p class="receipt">
          Active: {activeSanction.level}{activeSanction.expires_at
            ? ` — until ${activeSanction.expires_at}`
            : ' — no expiry'}
        </p>
      {/if}
    </div>

    {#if error}
      <p class="error">{error}</p>
    {/if}
  </div>
{/if}

<style>
  .moderation-panel {
    margin: 1rem 0;
    padding: 1rem;
    border: 1px solid var(--border, #ddd);
    border-radius: 0.5rem;
    background: var(--surface-muted, #f9f9f9);
  }
  .control {
    margin-bottom: 0.75rem;
  }
  label {
    display: block;
    font-weight: 600;
    margin-bottom: 0.25rem;
  }
  input,
  select {
    margin-right: 0.5rem;
    padding: 0.25rem;
  }
  .receipt {
    color: var(--ok, #2a7a2a);
    font-size: 0.875rem;
  }
  .error {
    color: var(--error, #c00);
    font-size: 0.875rem;
  }
</style>