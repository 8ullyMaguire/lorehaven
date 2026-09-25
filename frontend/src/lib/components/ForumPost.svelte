<script lang="ts">
  /**
   * Forum post with spoiler zone handling (spec §35.4).
   *
   * A post may carry content warnings that the reader has set prefs for
   * (blur or show). Blurred content is hidden behind a reveal button until
   * the reader chooses to see it.
   */
  import { getContentWarnings, type ContentWarning } from '../api';

  let { post }: { post: any } = $props();

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

  /**
   * The server returns warning_type, severity and custom_text only. There is
   * no per-warning id and no `action` field, so the identity a warning is
   * keyed and revealed by is (warning_type, severity).
   */
  function warningKey(warning: ContentWarning): string {
    return `${warning.warning_type}:${warning.severity}`;
  }

  function warningLabel(warning: ContentWarning): string {
    return warning.custom_text ?? warning.warning_type;
  }

  function shouldBlur(warning: ContentWarning): boolean {
    return warning.severity > 0;
  }

  let revealed = $state<Record<string, boolean>>({});

  // Derived: are there any active spoiler warnings?
  let hasSpoilers = $derived(
    !loading && warnings.some((w) => isSpoiler(w) && shouldBlur(w) && !revealed[warningKey(w)])
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
    {#if error}
      <p class="warning-error" role="alert">
        This post's content warnings could not be loaded, so it is shown unwarned.
      </p>
    {/if}
    {#each warnings.filter(isSpoiler) as warning (warningKey(warning))}
      {#if shouldBlur(warning) && !revealed[warningKey(warning)]}
        <div class="spoiler-zone">
          <button class="reveal-btn" onclick={() => revealed[warningKey(warning)] = true}>
            ⚠ {warningLabel(warning)} — click to reveal (spoilers ahead)
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
  .warning-error {
    color: var(--danger, #b91c1c);
    font-size: 0.875rem;
    margin: 0 0 0.75rem;
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
</style>