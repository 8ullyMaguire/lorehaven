<script lang="ts">
  interface Props {
    /** How many shimmering lines to draw. */
    lines?: number;
    /** Announced to assistive technology while content loads. */
    label?: string;
  }

  let { lines = 3, label = 'Loading' }: Props = $props();
</script>

<div class="skeleton" role="status" aria-busy="true">
  <span class="visually-hidden">{label}</span>
  {#each Array(lines) as _, index (index)}
    <div class="line" style={`--line-index:${index}`} aria-hidden="true"></div>
  {/each}
</div>

<style>
  .skeleton {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .line {
    height: 0.9rem;
    border-radius: var(--radius-sm);
    background: linear-gradient(
      90deg,
      var(--color-border) 0%,
      var(--color-surface) 50%,
      var(--color-border) 100%
    );
    background-size: 200% 100%;
    animation: shimmer 1.4s ease-in-out infinite;
    /* Last line shorter, like a paragraph. */
    width: calc(100% - var(--line-index) * 6%);
  }

  @keyframes shimmer {
    0% {
      background-position: 200% 0;
    }
    100% {
      background-position: -200% 0;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .line {
      animation: none;
    }
  }
</style>
