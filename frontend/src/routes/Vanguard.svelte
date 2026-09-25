<script lang="ts">
  import {
    fetchMyStreak,
    fetchPinsForWork,
    fetchVanguardStatus,
    fetchVanguards,
    pinWork,
    unpinWork,
    type Pin,
    type VanguardStatus,
  } from '../lib/api';
      import Button from '../lib/components/Button.svelte';
  import ErrorSummary from '../lib/components/ErrorSummary.svelte';
  import Skeleton from '../lib/components/Skeleton.svelte';

  let status = $state<VanguardStatus | null>(null);
  let vanguards = $state<string[]>([]);
  let pins = $state<Pin[]>([]);
  let streak = $state<{
    current_streak: number;
    longest_streak: number;
    last_read_date: string | null;
  } | null>(null);
  let loading = $state(true);
  let error = $state<unknown>(null);
  let pinning = $state<string | null>(null);
  let showPinForm = $state(false);
  let pinReason = $state('');
  let pinMessage = $state('');
  let selectedWorkId = $state('');

  async function load() {
    loading = true;
    error = null;
    try {
      const [statusResult, vanguardsResult, pinsResult, streakResult] = await Promise.all([
        fetchVanguardStatus(),
        fetchVanguards(),
        fetchPinsForWork('all'),
        fetchMyStreak(),
      ]);
      status = statusResult;
      vanguards = vanguardsResult.vanguards ?? [];
      pins = pinsResult.pins ?? [];
      streak = streakResult;
    } catch (failure) {
      error = failure;
    } finally {
      loading = false;
    }
  }

  async function handlePin() {
    if (!selectedWorkId || !pinReason) return;
    pinning = selectedWorkId;
    try {
      await pinWork(selectedWorkId, {
        pin_reason: pinReason,
        message: pinMessage || undefined,
      });
      showPinForm = false;
      selectedWorkId = '';
      pinReason = '';
      pinMessage = '';
      await load();
    } catch (failure) {
      error = failure;
    } finally {
      pinning = null;
    }
  }

  async function handleUnpin(workId: string) {
    pinning = workId;
    try {
      await unpinWork(workId);
      await load();
    } catch (failure) {
      error = failure;
    } finally {
      pinning = null;
    }
  }

  $effect(() => {
    void load();
  });
</script>

<section class="vanguard">
  <header class="vanguard-header">
    <h1>Taste Vanguard</h1>
    <p class="lede">
      Trusted curators who shape the platform's taste. Vanguards pin works to
      the Vanguard Picks shelf and help surface quality fiction.
    </p>
  </header>

  {#if error}
    <ErrorSummary {error} onretry={load} />
  {/if}

  {#if loading}
    <Skeleton lines={6} label="Loading" />
  {:else}
    {#if streak}
      <div class="vanguard-streak">
        <span class="streak-number">{streak.current_streak}</span>
        <span class="streak-label">day streak</span>
      </div>
    {/if}

    {#if status?.is_vanguard}
      <div class="vanguard-badge">
        <span class="badge-star">★</span>
        <span>You are a Vanguard</span>
      </div>

      <div class="vanguard-actions">
        {#if showPinForm}
          <div class="pin-form">
            <h3>Pin a work</h3>
            <label>
              Work ID
              <input type="text" bind:value={selectedWorkId} placeholder="work-uuid" />
            </label>
            <label>
              Reason
              <input type="text" bind:value={pinReason} placeholder="Why this work?" />
            </label>
            <label>
              Message (optional)
              <textarea bind:value={pinMessage} rows="3" placeholder="Optional note for readers"></textarea>
            </label>
            <div class="pin-form-actions">
              <Button onclick={handlePin} disabled={!selectedWorkId || !pinReason || pinning !== null}>
                {pinning ? 'Pinning...' : 'Pin work'}
              </Button>
              <button type="button" class="cancel" onclick={() => showPinForm = false}>
                Cancel
              </button>
            </div>
          </div>
        {:else}
          <Button onclick={() => showPinForm = true}>Pin a work</Button>
        {/if}
      </div>

      {#if pins.length > 0}
        <div class="vanguard-pins">
          <h2>Your pins</h2>
          <ul>
            {#each pins as pin}
              <li class="pin-item">
                <div class="pin-info">
                  <strong>{pin.work_id}</strong>
                  <span class="pin-reason">{pin.pin_reason}</span>
                  {#if pin.message}
                    <p class="pin-message">{pin.message}</p>
                  {/if}
                </div>
                <button
                  type="button"
                  class="unpin"
                  disabled={pinning === pin.work_id}
                  onclick={() => handleUnpin(pin.work_id)}
                >
                  {pinning === pin.work_id ? '...' : 'Unpin'}
                </button>
              </li>
            {/each}
          </ul>
        </div>
      {/if}
    {:else}
      <div class="vanguard-not">
        <p>You are not a Vanguard. Vanguards are appointed by the administrator based on contribution quality.</p>
      </div>
    {/if}

    {#if vanguards.length > 0}
      <div class="vanguard-list">
        <h2>Current vanguards</h2>
        <ul>
          {#each vanguards as vanguard}
            <li>{vanguard}</li>
          {/each}
        </ul>
      </div>
    {/if}
  {/if}
</section>

<style>
  .vanguard-header {
    margin-bottom: var(--space-6);
  }

  .vanguard-header h1 {
    margin-bottom: var(--space-2);
  }

  .lede {
    color: var(--text-muted);
    max-width: 60ch;
  }

  .vanguard-streak {
    display: flex;
    align-items: baseline;
    gap: var(--space-2);
    margin-bottom: var(--space-4);
    padding: var(--space-4);
    background: var(--surface);
    border-radius: var(--radius);
    border: 1px solid var(--border);
  }

  .streak-number {
    font-size: var(--text-2xl);
    font-weight: 700;
    color: var(--accent);
  }

  .streak-label {
    color: var(--text-muted);
  }

  .vanguard-badge {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-3) var(--space-4);
    background: var(--accent-subtle);
    border-radius: var(--radius);
    margin-bottom: var(--space-4);
  }

  .badge-star {
    color: var(--accent);
    font-size: var(--text-lg);
  }

  .vanguard-actions {
    margin-bottom: var(--space-6);
  }

  .pin-form {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding: var(--space-4);
    background: var(--surface);
    border-radius: var(--radius);
    border: 1px solid var(--border);
  }

  .pin-form h3 {
    margin: 0;
  }

  .pin-form label {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    font-size: var(--text-sm);
    color: var(--text-muted);
  }

  .pin-form input,
  .pin-form textarea {
    padding: var(--space-2);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--background);
    color: var(--text);
  }

  .pin-form-actions {
    display: flex;
    gap: var(--space-3);
    align-items: center;
  }

  .cancel {
    background: none;
    border: none;
    color: var(--text-muted);
    cursor: pointer;
    text-decoration: underline;
  }

  .vanguard-pins,
  .vanguard-list {
    margin-top: var(--space-6);
  }

  .vanguard-pins h2,
  .vanguard-list h2 {
    margin-bottom: var(--space-3);
  }

  .vanguard-pins ul,
  .vanguard-list ul {
    list-style: none;
    padding: 0;
    margin: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .pin-item {
    display: flex;
    justify-content: space-between;
    align-items: flex-start;
    padding: var(--space-3);
    background: var(--surface);
    border-radius: var(--radius);
    border: 1px solid var(--border);
  }

  .pin-info {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .pin-reason {
    font-size: var(--text-sm);
    color: var(--text-muted);
  }

  .pin-message {
    margin: 0;
    font-size: var(--text-sm);
  }

  .unpin {
    padding: var(--space-1) var(--space-2);
    background: var(--danger);
    color: white;
    border: none;
    border-radius: var(--radius);
    cursor: pointer;
    font-size: var(--text-sm);
  }

  .vanguard-not {
    padding: var(--space-4);
    background: var(--surface);
    border-radius: var(--radius);
    border: 1px solid var(--border);
    color: var(--text-muted);
  }
</style>
