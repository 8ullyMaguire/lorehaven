<script lang="ts">
  import { skipQuiz, saveQuizAnswers, fetchQuizWorks, type DiscoveryItem } from '../lib/api';
    import Button from '../lib/components/Button.svelte';
  import ErrorSummary from '../lib/components/ErrorSummary.svelte';
  import Skeleton from '../lib/components/Skeleton.svelte';

  let works = $state<DiscoveryItem[]>([]);
  let picked = $state<Set<string>>(new Set());
  let rejected = $state<Set<string>>(new Set());
  let loading = $state(true);
  let saving = $state(false);
  let error = $state<unknown>(null);

  async function load() {
    loading = true;
    error = null;
    try {
      const result = await fetchQuizWorks();
      works = result.works ?? [];
    } catch (failure) {
      error = failure;
    } finally {
      loading = false;
    }
  }

  function togglePicked(id: string) {
    if (picked.has(id)) {
      picked.delete(id);
    } else {
      picked.add(id);
      rejected.delete(id);
    }
    picked = new Set(picked);
    rejected = new Set(rejected);
  }

  function toggleRejected(id: string) {
    if (rejected.has(id)) {
      rejected.delete(id);
    } else {
      rejected.add(id);
      picked.delete(id);
    }
    picked = new Set(picked);
    rejected = new Set(rejected);
  }

  async function submit() {
    saving = true;
    try {
      await saveQuizAnswers({
        picked: Array.from(picked),
        rejected: Array.from(rejected),
      });
      window.location.href = '/discover';
    } catch (failure) {
      error = failure;
      saving = false;
    }
  }

  async function handleSkip() {
    await skipQuiz();
    window.location.href = '/discover';
  }

  $effect(() => {
    void load();
  });
</script>

<section class="quiz">
  <header class="quiz-header">
    <h1>Pick a few works you like</h1>
    <p class="lede">
      We'll use these to build your taste profile. You can change this later
      from your Discover page. Skip anytime — you'll still get a neutral
      feed.
    </p>
  </header>

  {#if error}
    <ErrorSummary {error} onretry={load} />
  {/if}

  {#if loading}
    <Skeleton lines={6} label="Loading questions" />
  {:else if works.length === 0}
    <p class="empty">No quiz works available yet. Import or publish some works first.</p>
  {:else}
    <ul class="quiz-works">
      {#each works as work}
        <li class="quiz-work">
          <div class="quiz-work-info">
            <strong>{work.title ?? 'Untitled'}</strong>
            {#if work.author_handle}
              <span class="quiz-author">{work.author_handle}</span>
            {/if}
            {#if work.word_count}
              <span class="quiz-word-count">{work.word_count} words</span>
            {/if}
          </div>
          <div class="quiz-actions">
            <button
              type="button"
              class="quiz-pick"
              class:active={picked.has(work.work_id)}
              onclick={() => togglePicked(work.work_id)}
            >
              Like
            </button>
            <button
              type="button"
              class="quiz-reject"
              class:active={rejected.has(work.work_id)}
              onclick={() => toggleRejected(work.work_id)}
            >
              Not for me
            </button>
          </div>
        </li>
      {/each}
    </ul>

    <footer class="quiz-footer">
      <span class="quiz-count">{picked.size} picked</span>
      <Button onclick={submit} disabled={picked.size + rejected.size === 0} loading={saving}>
        {saving ? 'Saving...' : 'Continue'}
      </Button>
      <button type="button" class="quiz-skip" onclick={handleSkip}>
        Skip for now
      </button>
    </footer>
  {/if}
</section>

<style>
  .quiz-header {
    margin-bottom: var(--space-6);
  }

  .quiz-header h1 {
    margin-bottom: var(--space-2);
  }

  .lede {
    color: var(--text-muted);
    max-width: 60ch;
  }

  .quiz-works {
    list-style: none;
    padding: 0;
    margin: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .quiz-work {
    border: 1px solid var(--border);
    border-radius: var(--radius);
    padding: var(--space-4);
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: var(--space-4);
  }

  .quiz-work-info {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .quiz-author,
  .quiz-word-count {
    font-size: var(--text-sm);
    color: var(--text-muted);
  }

  .quiz-actions {
    display: flex;
    gap: var(--space-2);
  }

  .quiz-pick,
  .quiz-reject {
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius);
    border: 1px solid var(--border);
    cursor: pointer;
    background: var(--surface);
    color: var(--text);
    transition: all 0.15s;
  }

  .quiz-pick.active {
    background: var(--accent);
    color: white;
    border-color: var(--accent);
  }

  .quiz-reject.active {
    background: var(--danger);
    color: white;
    border-color: var(--danger);
  }

  .quiz-footer {
    margin-top: var(--space-6);
    display: flex;
    align-items: center;
    gap: var(--space-4);
  }

  .quiz-count {
    color: var(--text-muted);
    font-size: var(--text-sm);
  }

  .quiz-skip {
    margin-left: auto;
    background: none;
    border: none;
    color: var(--text-muted);
    text-decoration: underline;
    cursor: pointer;
  }

  .empty {
    color: var(--text-muted);
    padding: var(--space-8) 0;
    text-align: center;
  }
</style>
