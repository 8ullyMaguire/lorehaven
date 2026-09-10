<script lang="ts">
  /**
   * "Resume where you left off", shown on a work page to a signed-in reader.
   *
   * Spec §9.3: when two devices disagree the reader is offered a *choice*,
   * never a silently-chosen position. So this component renders one link when
   * the positions agree and two when they do not, and it says where each one
   * is rather than asking the reader to trust a percentile.
   */
  import type { ProgressResolution, ServerPosition } from '../api';
  import { handleLinkClick } from '../router';

  interface Props {
    workId: string;
    resolution: ProgressResolution;
    /** The first chapter, used when a position names no chapter. */
    firstChapterId: string | null;
  }

  let { workId, resolution, firstChapterId }: Props = $props();

  function href(chapterId: string | null, position: ServerPosition): string {
    const target = chapterId ?? firstChapterId;
    if (!target) return `/works/${workId}`;
    return `/works/${workId}/chapters/${target}#p-${position.position_permille}`;
  }

  function describe(position: ServerPosition): string {
    const percent = Math.round(position.position_permille / 10);
    return position.device ? `${percent}% — on ${position.device}` : `${percent}% in`;
  }
</script>

{#if resolution.kind === 'use_stored' && resolution.position}
  <!--
    `{@const}`, not `!`. A `let` binding is not narrowed inside an `onclick`
    closure, and the two branches below already worked around that with non-null
    assertions. A `const` binding keeps the narrowing from the `{#if}` above it,
    which is the same guarantee without the assertion.
  -->
  {@const stored = resolution.position}
  <aside class="resume" aria-label="Resume reading">
    <p>
      You were <strong>{describe(stored)}</strong> last time you read this.
    </p>
    <a
      class="resume-link"
      href={href(null, stored)}
      onclick={(event) => handleLinkClick(event, href(null, stored))}
    >
      Resume reading
    </a>
  </aside>
{:else if resolution.kind === 'ask_the_reader'}
  <!-- Two devices disagree. Both are offered; neither is chosen for the reader. -->
  <aside class="resume" aria-label="Resume reading">
    <p>You stopped in two places. Which one would you like?</p>
    <ul>
      {#if resolution.mine}
        {@const mine = resolution.mine}
        <li>
          <a href={href(null, mine)} onclick={(event) => handleLinkClick(event, href(null, mine))}>
            {describe(mine)}
          </a>
        </li>
      {/if}
      {#if resolution.other}
        {@const other = resolution.other}
        <li>
          <a href={href(null, other)} onclick={(event) => handleLinkClick(event, href(null, other))}>
            {describe(other)}
          </a>
        </li>
      {/if}
    </ul>
  </aside>
{/if}

<style>
  .resume {
    background: var(--color-surface);
    border: var(--border-width) solid var(--color-border);
    border-radius: var(--radius-md);
    padding: var(--space-3) var(--space-4);
    margin-bottom: var(--space-4);
  }

  .resume p {
    margin: 0 0 var(--space-2);
    font-size: var(--text-sm);
  }

  .resume ul {
    margin: 0;
    padding-left: var(--space-4);
  }

  .resume-link {
    font-weight: 600;
  }
</style>
