<script lang="ts">
  /**
   * Forum post with spoiler zone handling (spec §35.4).
   *
   * A post may carry content warnings that the reader has set prefs for
   * (blur or show). Blurred content is hidden behind a reveal button until
   * the reader chooses to see it.
   */
  import {
    addContentWarning,
    getContentWarnings,
    type ContentWarning,
  } from '../api';

  let { post, readerChapter }: { post: any; readerChapter: number } = $props();

  let warnings = $state<ContentWarning[]>([]);
  let loading = $state(true);
  let error = $state<unknown>(null);

  async function loadWarnings() {
    loading = true;
    try {
      warnings = await getContentWarnings(post.id);
    } catch (failure) {
      error = failure;
    } finally {
      loading = false;
    }
  }

  void loadWarnings();

  function isSpoiler(warning: ContentWarning): boolean {
    return warning.warning_type === 'spoiler';
  }

  function shouldBlur(warning: ContentWarning): boolean {
    if (warning.chapter_threshold && readerChapter < warning.chapter_threshold) {
      return true;
    }
    return warning.action === 'blur';
  }

  let revealed = $state<Record<string, boolean>>({});

  // Derived: are there any active spoiler warnings?
  let hasSpoilers = $derived(
    !loading && warnings.some((w) => isSpoiler(w) && shouldBlur(w) && !revealed[w.id])
  );
</script>

<div class="post" data-testid="forum-post">
  <header class="post-meta">
    <span class="author">{post.author_handle ?? post.author_pseud}</span>
    <span class="when">{new Date(post.created_at).toLocaleString()}</span>
  </header>

  {#if loading}
    <p>Loading…</p>
  {:else}
    {#each warnings.filter(isSpoiler) as warning (warning.id)}
      {#if shouldBlur(warning) && !revealed[warning.id]}
        <div class="spoiler-zone">
          <button class="reveal-btn" onclick={() => revealed[warning.id] = true}>
            ⚠ {warning.label} — click to reveal (spoilers ahead)
          </button>
        </div>
      {/if}
    {/each}

    {#if hasSpoilers}
      <!-- Body is hidden behind spoiler zones above -->
    {:else}
      <div class="post-body">
        {@html post.body}
      </div>
    {/if}
  {/if}
</div>

<style>
  .post {
    margin-bottom: 1rem;
    padding: 0.75rem;
    border: 1px solid var(--border, #ddd);
    border-radius: 0.5rem;
  }
  .post-meta {
    font-size: 0.875rem;
    color: var(--text-muted, #666);
    margin-bottom: 0.5rem;
  }
  .author {
    font-weight: 600;
    margin-right: 0.5rem;
  }
  .spoiler-zone {
    padding: 1rem;
    background: var(--surface-muted, #f5f5f5);
    border-radius: 0.5rem;
    text-align: center;
    margin-bottom: 0.5rem;
  }
  .reveal-btn {
    cursor: pointer;
    padding: 0.5rem 1rem;
    border: 1px solid var(--border, #ddd);
    border-radius: 0.25rem;
    background: var(--accent-bg, #eef);
  }
  .post-body {
    line-height: 1.6;
  }
  .warning-tag {
    display: inline-block;
    margin-top: 0.5rem;
    padding: 0.25rem 0.5rem;
    background: var(--accent-bg, #eef);
    border-radius: 0.25rem;
    font-size: 0.75rem;
  }
</style>