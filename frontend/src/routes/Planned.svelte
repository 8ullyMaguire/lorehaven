<script lang="ts">
  import EmptyState from '../lib/components/EmptyState.svelte';
  import MetadataChip from '../lib/components/MetadataChip.svelte';
  import type { PlannedRoute } from '../lib/router';

  interface Props {
    route: PlannedRoute;
  }

  let { route }: Props = $props();
</script>

<article class="planned">
  <header>
    <MetadataChip label={route.milestone} tone="accent" />
    <h1>{route.title}</h1>
  </header>

  <p class="summary">{route.summary}</p>

  <EmptyState
    title="Not built yet"
    description="This page exists so the navigation is honest about where it leads. It will be filled in by the milestone above, backed by real endpoints — not by placeholder data."
  />
</article>

<style>
  .planned {
    padding-top: var(--space-6);
  }

  header {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--space-3);
  }

  header h1 {
    margin: 0;
  }

  .summary {
    font-size: var(--text-lg);
    color: var(--color-muted);
    max-width: 52ch;
  }
</style>
