<script lang="ts">
  import {
    fetchContentSettings,
    fetchFeedbackInbox,
    fetchFeedbackPreferences,
    fetchPrivacy,
    fetchSessions,
    patchContentSettings,
    patchPrivacy,
    putFeedbackPreferences,
    revokeAllSessions,
    revokeSession,
    type ContentSettings,
    type FeedbackInbox,
    type FeedbackPreferencesView,
    // Aliased: `PrivacySettings` below is the component that renders them.
    type PrivacySettings as PrivacySettingsResponse,
    type SessionSummary,
  } from '../lib/api';
  import { describeAgeState } from '../lib/labels';
  import { handleLinkClick } from '../lib/router';
  import { session } from '../lib/session.svelte';
  import { formatTimestamp } from '../lib/time';
  import Button from '../lib/components/Button.svelte';
  import ContentPreferences from '../lib/components/ContentPreferences.svelte';
  import EmptyState from '../lib/components/EmptyState.svelte';
  import ErrorSummary from '../lib/components/ErrorSummary.svelte';
  import PrivacySettings from '../lib/components/PrivacySettings.svelte';
  import Skeleton from '../lib/components/Skeleton.svelte';
  import Tabs from '../lib/components/Tabs.svelte';
  import ToastRegion from '../lib/components/ToastRegion.svelte';

  /** Structurally the shape `ToastRegion` accepts. */
  interface Notice {
    id: string;
    message: string;
    tone?: 'info' | 'success' | 'danger';
  }

  const TABS = [
    { id: 'sessions', label: 'Sessions' },
    { id: 'reading', label: 'Reading' },
    { id: 'feedback', label: 'Feedback' },
    { id: 'privacy', label: 'Privacy' },
  ];

  let tab = $state('sessions');

  let sessions = $state<SessionSummary[] | null>(null);
  let privacy = $state<PrivacySettingsResponse | null>(null);
  let content = $state<ContentSettings | null>(null);
  let feedback = $state<FeedbackPreferencesView | null>(null);
  let inbox = $state<FeedbackInbox | null>(null);

  let error = $state<unknown>(null);
  let loading = $state(false);
  let signingOut = $state(false);
  let toasts = $state<Notice[]>([]);

  let toastCounter = 0;
  function say(message: string, tone: Notice['tone'] = 'info') {
    toastCounter += 1;
    toasts = [...toasts, { id: `toast-${toastCounter}`, message, tone }];
  }

  /**
   * Everything on this page belongs to the signed-in account, so it is loaded
   * only once there is a session — and re-loaded whenever the session changes,
   * which is what makes a pseud switch elsewhere show up here.
   */
  $effect(() => {
    if (session.isSignedIn) void load();
  });

  async function load() {
    loading = true;
    error = null;
    try {
      const [sessionList, privacySettings, contentSettings, feedbackPrefs, feedbackInbox] =
        await Promise.all([
          fetchSessions(),
          fetchPrivacy(),
          fetchContentSettings(),
          fetchFeedbackPreferences(),
          fetchFeedbackInbox(),
        ]);
      sessions = sessionList;
      privacy = privacySettings;
      content = contentSettings;
      feedback = feedbackPrefs;
      inbox = feedbackInbox;
    } catch (failure) {
      error = failure;
    } finally {
      loading = false;
    }
  }

  async function signOut() {
    signingOut = true;
    try {
      await session.signOutNow();
      sessions = null;
      privacy = null;
      content = null;
      feedback = null;
      inbox = null;
    } finally {
      signingOut = false;
    }
  }

  async function dropOne(summary: SessionSummary) {
    error = null;
    try {
      await revokeSession(summary.id);
      await load();
      say(`Signed out ${summary.device}.`, 'success');
    } catch (failure) {
      error = failure;
    }
  }

  async function dropAll() {
    error = null;
    try {
      await revokeAllSessions();
      // The request that revoked them was itself authenticated; the account is
      // now signed out and the store has to be told.
      await session.refresh();
      say('Signed out everywhere.', 'success');
    } catch (failure) {
      error = failure;
    }
  }

  /** Account-scoped privacy keys only: pseud-level ones live on the pseud pages. */
  let accountKeys = $derived(
    (privacy?.schema ?? []).filter((key) => key.key in (privacy?.account ?? {})),
  );

  let capabilities = $derived(session.me?.capabilities ?? null);
</script>

{#if !session.isSignedIn}
  <section class="page">
    <h1>Your account</h1>
    {#if session.error}
      <ErrorSummary error={session.error} onretry={() => session.refresh()} />
    {/if}
    <EmptyState
      title="Sign in to see this page"
      description="Sessions, reading preferences and privacy settings all belong to an account. Reading does not require one."
    >
      {#snippet action()}
        <div class="actions">
          <a class="inline-link" href="/sign-in" onclick={(event) => handleLinkClick(event, '/sign-in')}>
            Sign in
          </a>
          <a class="inline-link" href="/register" onclick={(event) => handleLinkClick(event, '/register')}>
            Create an account
          </a>
        </div>
      {/snippet}
    </EmptyState>
  </section>
{:else}
  <section class="page">
    <header class="head">
      <div>
        <h1>Your account</h1>
        {#if session.me}
          <p class="who">
            {session.me.account.email} ·
            <span class="age">{describeAgeState(session.me.account.age_state)}</span>
          </p>
        {/if}
      </div>
      <Button variant="secondary" loading={signingOut} onclick={signOut}>Sign out</Button>
    </header>

    {#if capabilities?.restriction_note}
      <p class="restriction" role="status">{capabilities.restriction_note}</p>
    {/if}

    {#if error}
      <ErrorSummary {error} onretry={load} />
    {/if}

    <Tabs tabs={TABS} value={tab} onchange={(id) => (tab = id)}>
      {#snippet children(id)}
        {#if id === 'sessions'}
          <div class="panel">
            {#if loading && !sessions}
              <Skeleton lines={4} label="Loading your sessions" />
            {:else if sessions}
              <p class="note">
                Every device signed in to this account. Revoking one takes effect on the
                next request that device makes.
              </p>
              <ul class="sessions">
                {#each sessions as summary (summary.id)}
                  <li>
                    <div class="device">
                      <strong>{summary.device}</strong>
                      {#if summary.current}
                        <span class="badge">This device</span>
                      {/if}
                    </div>
                    <dl class="facts">
                      <dt>Signed in</dt>
                      <dd>{formatTimestamp(summary.created_at)}</dd>
                      <dt>Last active</dt>
                      <dd>{formatTimestamp(summary.last_seen_at)}</dd>
                      <dt>Expires</dt>
                      <dd>{formatTimestamp(summary.expires_at)}</dd>
                    </dl>
                    {#if summary.current}
                      <Button variant="quiet" size="sm" loading={signingOut} onclick={signOut}>
                        Sign out this device
                      </Button>
                    {:else}
                      <Button variant="secondary" size="sm" onclick={() => dropOne(summary)}>
                        Revoke
                      </Button>
                    {/if}
                  </li>
                {/each}
              </ul>

              {#if sessions.length > 1}
                <div class="danger-zone">
                  <h3>Sign out everywhere</h3>
                  <p>
                    Ends every session, including this one. Use it if you think somebody
                    else has your password.
                  </p>
                  <Button variant="danger" size="sm" onclick={dropAll}>
                    Sign out everywhere
                  </Button>
                </div>
              {/if}
            {/if}
          </div>
        {:else if id === 'reading'}
          <div class="panel">
            {#if loading && !content}
              <Skeleton lines={4} label="Loading your reading preferences" />
            {:else if content}
              <ContentPreferences
                settings={content}
                onsave={async (patch) => {
                  content = await patchContentSettings(patch);
                  say('Reading preferences saved.', 'success');
                }}
                onreload={load}
              />
              <p class="note">
                What you are shown is the lower of your preference and the instance
                policy. Both are reported above so a setting that has no effect says so.
              </p>
            {/if}
          </div>
        {:else if id === 'feedback'}
          <div class="panel">
            {#if loading && !feedback}
              <Skeleton lines={4} label="Loading your feedback preferences" />
            {:else if feedback}
              <p class="note" role="status">{feedback.effective_policy}</p>
              <label class="share">
                <input
                  type="checkbox"
                  checked={feedback.accept_constructive}
                  onchange={async (event) => {
                    const accept_constructive = (event.target as HTMLInputElement).checked;
                    try {
                      feedback = await putFeedbackPreferences({
                        accept_constructive,
                        expected_version: feedback?.version ?? 0,
                      });
                      say('Feedback preferences saved.', 'success');
                    } catch (failure) {
                      error = failure;
                    }
                  }}
                />
                Receive constructive critique (opt-in, off by default)
              </label>
              <label class="share">
                <input
                  type="checkbox"
                  checked={feedback.comments_enabled}
                  onchange={async (event) => {
                    const comments_enabled = (event.target as HTMLInputElement).checked;
                    try {
                      feedback = await putFeedbackPreferences({
                        comments_enabled,
                        expected_version: feedback?.version ?? 0,
                      });
                      say('Feedback preferences saved.', 'success');
                    } catch (failure) {
                      error = failure;
                    }
                  }}
                />
                Receive new feedback at all (pausing holds it, never deletes it)
              </label>
              {#if inbox}
                <h3>Received feedback</h3>
                {#if inbox.items.length === 0}
                  <p class="note">No delivered feedback yet.</p>
                {:else}
                  <ul class="sessions">
                    {#each inbox.items as item (item.review_id)}
                      <li>
                        <div class="device">
                          <strong>@{item.author_handle}</strong>
                          <span class="badge">{item.class}</span>
                        </div>
                        <p>{item.body}</p>
                        <p class="note">On {item.work_title}</p>
                      </li>
                    {/each}
                  </ul>
                {/if}
                {#if inbox.held_count > 0}
                  <p class="note" role="status">
                    {inbox.held_count} held for review. Held text is counted, never shown.
                  </p>
                {/if}
              {/if}
            {/if}
          </div>
        {:else}
          <div class="panel">
            {#if loading && !privacy}
              <Skeleton lines={4} label="Loading your privacy settings" />
            {:else if privacy}
              <PrivacySettings
                legend="Who can see what"
                note="These apply to the account as a whole. Settings that belong to one pseud are edited on that pseud's page."
                keys={accountKeys}
                values={privacy.account}
                onsave={async (changes) => {
                  privacy = await patchPrivacy(changes);
                  say('Privacy settings saved.', 'success');
                }}
              />
            {/if}
          </div>
        {/if}
      {/snippet}
    </Tabs>
  </section>
{/if}

<ToastRegion {toasts} ondismiss={(id) => (toasts = toasts.filter((t) => t.id !== id))} />

<style>
  .page {
    max-width: 52rem;
  }

  .head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--space-4);
    flex-wrap: wrap;
    margin-bottom: var(--space-4);
  }

  h1 {
    font-family: var(--font-heading);
    margin: 0 0 var(--space-2);
  }

  .who {
    margin: 0;
    color: var(--color-muted);
    font-size: var(--text-sm);
  }

  .restriction {
    border-left: 3px solid var(--color-accent);
    background: var(--color-accent-soft);
    padding: var(--space-3);
    border-radius: var(--radius-sm);
    margin: 0 0 var(--space-4);
  }

  .panel {
    padding-top: var(--space-4);
  }

  .note {
    color: var(--color-muted);
    font-size: var(--text-sm);
    max-width: 60ch;
  }

  .sessions {
    list-style: none;
    padding: 0;
    margin: 0 0 var(--space-5);
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .sessions li {
    border: var(--border-width) solid var(--color-border);
    border-radius: var(--radius-md);
    padding: var(--space-4);
    background: var(--color-surface);
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-4);
  }

  .device {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 12rem;
  }

  .badge {
    font-size: var(--text-xs);
    background: var(--color-accent-soft);
    border: var(--border-width) solid var(--color-accent);
    border-radius: 999px;
    padding: 0 var(--space-2);
    color: var(--color-text);
  }

  .facts {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: var(--space-1) var(--space-3);
    margin: 0;
    flex: 1;
    font-size: var(--text-sm);
  }

  .facts dt {
    color: var(--color-muted);
  }

  .facts dd {
    margin: 0;
  }

  .danger-zone {
    border: var(--border-width) solid var(--color-danger);
    border-radius: var(--radius-md);
    padding: var(--space-4);
    max-width: 40rem;
  }

  .danger-zone h3 {
    margin-top: 0;
  }

  .danger-zone p {
    color: var(--color-muted);
    font-size: var(--text-sm);
    margin-top: 0;
  }

  .actions {
    display: flex;
    align-items: center;
    gap: var(--space-4);
  }
</style>
