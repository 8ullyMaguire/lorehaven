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
  import { session } from '../lib/session.svelte.ts';
  import Button from '../lib/components/Button.svelte';
  import ErrorSummary from '../lib/components/ErrorSummary.svelte';
  import SignInGate from '../lib/components/SignInGate.svelte';
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
  /**
   * Set when the operator-only roster refused us.
   *
   * Its own flag rather than a string, because the distinction it records is not
   * "did it fail" — a 403 there is the expected answer for a normal reader, and
   * rendering it as an error is the defect this page had.
   */
  let vanguardsRefused = $state(false);
  let pinning = $state<string | null>(null);
  let showPinForm = $state(false);
  let pinReason = $state('');
  let pinMessage = $state('');
  let selectedWorkId = $state('');

  /**
   * Why this is `Promise.allSettled` and not `Promise.all`.
   *
   * Four independent panels shared one `Promise.all` and one `error` slot. One of
   * the four — `listVanguards()` → `GET /api/v1/vanguards` — is
   * `require_operator`, so it answers **403 to every account that is not the
   * operator**, and the whole page rendered "That did not work / access denied"
   * while the reader's own status, pins and streak were thrown away with it. The
   * "Current vanguards" section is a public roster the operator maintains; it is
   * simply not available to a normal reader, and its absence is not a failure of
   * anything on this page.
   *
   * `allSettled` keeps the four outcomes separate: each panel lands or does not,
   * and only a panel that actually failed sets an error. `vanguardsRefused` is
   * checked first so a 403 there is recognised as the roster being operator-only
   * rather than as a fault.
   */
  async function load() {
    loading = true;
    error = null;
    vanguardsRefused = false;
    const [statusResult, vanguardsResult, pinsResult, streakResult] = await Promise.allSettled([
      fetchVanguardStatus(),
      fetchVanguards(),
      fetchPinsForWork('all'),
      fetchMyStreak(),
    ]);

    if (statusResult.status === 'fulfilled') status = statusResult.value;
    if (pinsResult.status === 'fulfilled') pins = pinsResult.value.pins ?? [];
    if (streakResult.status === 'fulfilled') streak = streakResult.value;

    if (vanguardsResult.status === 'fulfilled') {
      vanguards = vanguardsResult.value.vanguards ?? [];
    } else {
      // Operator-only. The section renders as absent, which is the truth.
      vanguardsRefused = isForbidden(vanguardsResult.reason);
      if (!vanguardsRefused) error = vanguardsResult.reason;
    }

    // The reader's own three panels: a failure in any of them IS this page's
    // failure, and it is reported rather than silently dropped.
    for (const outcome of [statusResult, pinsResult, streakResult]) {
      if (outcome.status === 'rejected') {
        error = outcome.reason;
        break;
      }
    }

    loading = false;
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

  /**
   * A 403 or 404 from a door that exists but is not for this reader.
   *
   * Both codes mean "not for you" on this page: `listVanguards` answers 403 (it
   * is a trust-gated list), and a 404 is what a renamed door would answer. Neither
   * is something the reader did wrong or can fix, which is what separates it from
   * an error worth the red panel.
   */
  function isForbidden(failure: unknown): boolean {
    if (typeof failure !== 'object' || failure === null || !('status' in failure)) {
      return false;
    }
    const status = (failure as { status?: number }).status;
    return status === 403 || status === 404;
  }

  // Signed out, the roster, the pins and the streak are all this reader's own or
  // operator-only, so the page is a sign-in note rather than a 401 panel.
  $effect(() => {
    if (session.isSignedIn) void load();
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

  <SignInGate purpose="see your vanguard status, pins and streak" skeletonLines={6}>
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

    <!--
      Absent for a reader who is not the operator, with no apology for it: the
      roster is curated by the operator and this account is not shown it. The
      page above has already said whether the reader is a Vanguard.
    -->
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
  </SignInGate>
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
