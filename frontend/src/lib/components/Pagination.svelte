<script lang="ts">
  interface Props {
    /** Current page, 1-based. */
    page: number;
    /** Total number of pages; 0 or 1 renders nothing. */
    totalPages: number;
    onchange?: (page: number) => void;
  }

  let { page, totalPages, onchange }: Props = $props();

  /** A windowed list of page numbers with `null` marking a gap. */
  function windowed(current: number, total: number): (number | null)[] {
    if (total <= 7) return Array.from({ length: total }, (_, index) => index + 1);
    const pages = new Set<number>([1, total, current, current - 1, current + 1]);
    const sorted = [...pages].filter((p) => p >= 1 && p <= total).sort((a, b) => a - b);
    const result: (number | null)[] = [];
    let previous = 0;
    for (const value of sorted) {
      if (previous && value - previous > 1) result.push(null);
      result.push(value);
      previous = value;
    }
    return result;
  }

  let pages = $derived(windowed(page, totalPages));

  function go(next: number) {
    if (next < 1 || next > totalPages || next === page) return;
    onchange?.(next);
  }
</script>

{#if totalPages > 1}
  <nav class="pagination" aria-label="Pagination">
    <button type="button" class="step" disabled={page <= 1} onclick={() => go(page - 1)}>
      Previous
    </button>

    <ol>
      {#each pages as value, index (index)}
        {#if value === null}
          <li class="gap" aria-hidden="true">…</li>
        {:else}
          <li>
            <button
              type="button"
              class="page"
              aria-current={value === page ? 'page' : undefined}
              aria-label={`Page ${value}`}
              onclick={() => go(value)}
            >
              {value}
            </button>
          </li>
        {/if}
      {/each}
    </ol>

    <button type="button" class="step" disabled={page >= totalPages} onclick={() => go(page + 1)}>
      Next
    </button>
  </nav>
{/if}

<style>
  .pagination {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    flex-wrap: wrap;
    margin-top: var(--space-5);
  }

  ol {
    display: flex;
    gap: var(--space-1);
    list-style: none;
    margin: 0;
    padding: 0;
    align-items: center;
  }

  .page,
  .step {
    font: inherit;
    background: var(--color-surface);
    border: var(--border-width) solid var(--color-border);
    border-radius: var(--radius-sm);
    color: var(--color-text);
    padding: var(--space-2) var(--space-3);
    cursor: pointer;
    min-width: 2.5rem;
  }

  .page[aria-current='page'] {
    background: var(--color-primary);
    border-color: var(--color-primary);
    color: var(--color-primary-contrast);
    font-weight: 700;
  }

  .step:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .gap {
    color: var(--color-muted);
    padding: 0 var(--space-1);
  }
</style>
