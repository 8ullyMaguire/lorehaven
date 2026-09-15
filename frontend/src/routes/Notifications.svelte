<script lang="ts">
  /**
   * Notifications centre (spec §23).
   *
   * Outbox-driven delivery: library updates, job completions, replies, mentions.
   * Lock-screen text is generic (the work title, never content). Mark one or all
   * as read.
   */
  import {
    fetchNotifications,
    markAllNotificationsRead,
    markNotificationRead,
    type NotificationItem,
  } from '../lib/api';
  import { handleLinkClick } from '../lib/router';
  import Button from '../lib/components/Button.svelte';
  import ErrorSummary from '../lib/components/ErrorSummary.svelte';
  import Skeleton from '../lib/components/Skeleton.svelte';

  let items = $state<NotificationItem[]>([]);
  let unread = $state(0);
  let error = $state<unknown>(null);
  let loading = $state(true);
  let marking = $state(false);

  async function load() {
    loading = true;
    error = null;
    try {
      const result = await fetchNotifications();
      items = result.items ?? [];
      unread = result.unread_count ?? 0;
    } catch (failure) {
      error = failure;
    } finally {
      loading = false;
    }
  }

  async function markAll() {
    marking = true;
    try {
      await markAllNotificationsRead();
      items = items.map(n => ({ ...n, read: true }));
      unread = 0;
    } catch (failure) {
      error = failure;
    } finally {
      marking = false;
    }
  }

  async function markOne(id: string) {
    await markNotificationRead(id);
    const n = items.find(x => x.id === id);
    if (n && !n.read) {
      n.read = true;
      unread = Math.max(0, unread - 1);
    }
  }

  function timeAgo(iso: string): string {
    try {
      const diff = Date.now() - new Date(iso).getTime();
      const m = Math.floor(diff / 60000);
      if (m < 1) return 'just now';
      if (m < 60) return `${m}m ago`;
      const h = Math.floor(m / 60);
      if (h < 24) return `${h}h ago`;
      return `${Math.floor(h / 24)}d ago`;
    } catch {
      return iso;
    }
  }

  $effect(() => {
    void load();
  });
</script>

<section class="notifications">
  <header>
    <h1>Notifications</h1>
    {#if unread > 0}
      <span class="unread-badge">{unread} new</span>
    {/if}
  </header>

  {#if error}
    <ErrorSummary {error} onretry={load} />
  {/if}

  {#if loading}
    <Skeleton lines={6} label="Loading notifications" />
  {:else if items.length === 0}
    <p class="empty">No notifications yet.</p>
  {:else}
    <div class="actions">
      <Button variant="secondary" onclick={markAll} disabled={marking || unread === 0}>
        {marking ? 'Marking…' : 'Mark all as read'}
      </Button>
    </div>
    <ul class="notification-list">
      {#each items as item}
        <li class:read={item.read}>
          {#if item.work_id}
            {@const workId = item.work_id}
            <a
              href={`/works/${encodeURIComponent(workId)}`}
              onclick={(event) => handleLinkClick(event, `/works/${encodeURIComponent(workId)}`)}
            >
              <span class="kind">{item.kind}</span>
              <span class="title">{item.title}</span>
              <span class="time">{timeAgo(item.created_at)}</span>
            </a>
          {:else}
            <span class="notification-text">
              <span class="kind">{item.kind}</span>
              <span class="title">{item.title}</span>
              <span class="time">{timeAgo(item.created_at)}</span>
            </span>
          {/if}
          {#if !item.read}
            <button class="mark" onclick={() => markOne(item.id)}>Mark read</button>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
</section>

<style>
  .notifications {
    max-width: 72rem;
    margin-inline: auto;
    padding: 2rem 1rem;
  }
  header {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    margin-bottom: 1.5rem;
  }
  header h1 {
    margin: 0;
  }
  .unread-badge {
    background: var(--accent);
    color: white;
    border-radius: 999px;
    padding: 0.25rem 0.75rem;
    font-size: 0.75rem;
    font-weight: 600;
  }
  .empty {
    padding: 2rem;
    text-align: center;
    color: var(--text-muted);
    background: var(--surface-muted);
    border-radius: 0.5rem;
  }
  .actions {
    margin-bottom: 1rem;
  }
  .notification-list {
    list-style: none;
    padding: 0;
    margin: 0;
    display: grid;
    gap: 0.5rem;
  }
  .notification-list li {
    display: flex;
    align-items: center;
    gap: 1rem;
    padding: 1rem;
    border: 1px solid var(--border);
    border-radius: 0.5rem;
  }
  .notification-list li.read {
    opacity: 0.6;
  }
  .notification-list a,
  .notification-text {
    flex: 1;
    display: flex;
    flex-direction: column;
    text-decoration: none;
    color: inherit;
    gap: 0.25rem;
  }
  .kind {
    font-size: 0.75rem;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-muted);
  }
  .title {
    font-weight: 600;
  }
  .time {
    font-size: 0.75rem;
    color: var(--text-muted);
  }
  .mark {
    border: none;
    background: var(--surface-muted);
    border-radius: 0.25rem;
    padding: 0.25rem 0.5rem;
    cursor: pointer;
    font-size: 0.75rem;
    color: var(--text-muted);
  }
  .mark:hover {
    background: var(--border);
  }
</style>
