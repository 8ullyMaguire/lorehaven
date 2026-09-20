<script lang="ts">
  /**
   * Reading group schedule (spec §35.3).
   *
   * Shows a topic's reading schedule with dated unlocks. Used on topics
   * with mode=reading_group to show readers what to read and when.
   */
  import { addScheduleSection, getSchedule, type ScheduleSection } from '../api';

  let { topicId, isModerator }: { topicId: string; isModerator: boolean } = $props();

  let sections = $state<ScheduleSection[]>([]);
  let loading = $state(true);
  let error = $state<unknown>(null);

  // Form state
  let newTitle = $state('');
  let newStart = $state(1);
  let newEnd = $state(1);
  let newUnlocks = $state('');
  let adding = $state(false);

  async function load() {
    loading = true;
    error = null;
    try {
      sections = await getSchedule(topicId);
    } catch (failure) {
      error = failure;
    } finally {
      loading = false;
    }
  }

  async function add() {
    if (!newTitle || adding) return;
    adding = true;
    error = null;
    try {
      await addScheduleSection({
        topic_id: topicId,
        title: newTitle,
        chapter_start: newStart,
        chapter_end: newEnd,
        unlocks_at: newUnlocks || undefined,
      });
      newTitle = '';
      newUnlocks = '';
      await load();
    } catch (failure) {
      error = failure;
    } finally {
      adding = false;
    }
  }

  void load();
</script>

<div class="reading-schedule">
  <h4>Reading Schedule</h4>

  {#if loading}
    <p>Loading…</p>
  {:else if sections.length === 0}
    <p>No schedule yet.</p>
  {:else}
    <ul>
      {#each sections as section (section.position)}
        <li>
          <strong>{section.title}</strong>
          <span class="chapters">
            ch. {section.chapter_start}–{section.chapter_end}
          </span>
          {#if section.unlocks_at}
            <span class="unlocks">
              unlocks {new Date(section.unlocks_at).toLocaleDateString()}
            </span>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}

  {#if isModerator}
    <div class="add-section">
      <input type="text" bind:value={newTitle} placeholder="Section title" />
      <input type="number" bind:value={newStart} min="1" placeholder="From ch." />
      <input type="number" bind:value={newEnd} min="1" placeholder="To ch." />
      <input type="date" bind:value={newUnlocks} />
      <button onclick={add} disabled={adding || !newTitle}>
        {adding ? 'Adding…' : 'Add'}
      </button>
    </div>
  {/if}

  {#if error}
    <p class="error">{error}</p>
  {/if}
</div>

<style>
  .reading-schedule {
    margin: 1rem 0;
    padding: 0.75rem;
    border: 1px solid var(--border, #ddd);
    border-radius: 0.5rem;
  }
  ul {
    list-style: none;
    padding: 0;
  }
  li {
    margin-bottom: 0.5rem;
  }
  .chapters {
    margin-left: 0.5rem;
    color: var(--text-muted, #666);
  }
  .unlocks {
    margin-left: 0.5rem;
    font-size: 0.875rem;
    color: var(--text-muted, #666);
  }
  .add-section {
    display: flex;
    gap: 0.5rem;
    margin-top: 0.75rem;
    flex-wrap: wrap;
  }
  .error {
    color: var(--error, #c00);
    font-size: 0.875rem;
  }
</style>