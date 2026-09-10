<script lang="ts">
  /**
   * A long passage of text from somewhere else, shown a few lines at a time.
   *
   * Somewhere else is the point. A source's summary is not a blurb we wrote and
   * not necessarily a blurb at all: Royal Road's for Mother of Learning runs to
   * two thousand four hundred characters and ends in the author's Amazon,
   * Patreon and PayPal links. Listing that unclamped turned a library of twenty
   * works into twenty screens of one author's marketing, so the text is clamped
   * — but clamped for length, not deleted, because the reader may be deciding on
   * this very work and the full text is part of what they are deciding on.
   *
   * The disclosure only appears when there is something behind it. Clamping by
   * character count alone would put "Show more" under a two-line summary that
   * has nothing more to show, which trains a reader to ignore the control.
   */
  interface Props {
    text: string;
    /** Lines shown while collapsed. */
    lines?: number;
    /** Label for the expand control. Defaults to a generic pair. */
    moreLabel?: string;
    lessLabel?: string;
    class?: string;
  }

  let {
    text,
    lines = 4,
    moreLabel = 'Show more',
    lessLabel = 'Show less',
    class: className = '',
  }: Props = $props();

  let expanded = $state(false);
  /**
   * Whether the clamp is actually hiding something.
   *
   * Measured rather than guessed, and measured against the *clamped* element, so
   * it stays right when the reader resizes the window or the text reflows at a
   * different width. Starts false so the control is never shown before there is
   * a measurement to justify it.
   */
  let overflowing = $state(false);
  let paragraph = $state<HTMLParagraphElement | null>(null);

  $effect(() => {
    const element = paragraph;
    if (!element) return;

    // Read `text` and `lines` so the effect re-runs when either changes: the
    // element is the same node across an import refresh, so a measurement taken
    // for the previous work would otherwise be reused for the next one.
    void text;
    void lines;
    void expanded;

    const measure = () => {
      if (!element) return;
      overflowing = element.scrollHeight > element.clientHeight + 1;
    };

    measure();

    // The width can change without the text changing — a window resize, a
    // sidebar appearing — and the clamp is a function of both.
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  });
</script>

<div class="clamped-text {className}">
  <p
    bind:this={paragraph}
    class:clamped={!expanded}
    style="--clamp-lines: {lines}"
  >
    {text}
  </p>
  {#if overflowing || expanded}
    <button
      class="disclose"
      type="button"
      onclick={() => (expanded = !expanded)}
      aria-expanded={expanded}
    >
      {expanded ? lessLabel : moreLabel}
    </button>
  {/if}
</div>

<style>
  p {
    margin: 0;
    max-width: 68ch;
    /* `anywhere` because a source's summary can carry a URL with no spaces in
       it, and without this the clamp's own box grows to fit the unbreakable
       word instead of clamping it. */
    overflow-wrap: anywhere;
  }

  p.clamped {
    display: -webkit-box;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: var(--clamp-lines, 4);
    /* The standard property alongside the prefixed one, which is what
       svelte-check asks for and what the engines that have shipped the
       standard spelling read. */
    line-clamp: var(--clamp-lines, 4);
    overflow: hidden;
  }

  .disclose {
    background: none;
    border: 0;
    padding: 0;
    margin-top: var(--space-1);
    color: var(--color-accent);
    cursor: pointer;
    font: inherit;
    text-decoration: underline;
  }
</style>
