<script lang="ts">
  /**
   * The docs pages: `/docs` (index) and `/docs/{slug}` (one page).
   *
   * Content comes from the bundled registry (`../lib/docs`) — plain
   * markdown written for the lowest common denominator, rendered with
   * `marked`. Markdown here is authored in-repo (trusted content, like the
   * server-sanitized chapter HTML the Reader renders); user content never
   * passes through this path.
   */
  import { marked } from 'marked';
  import { DOCS, docBySlug } from '../lib/docs';

  let { slug = '' }: { slug?: string } = $props();

  const doc = $derived(slug === '' ? undefined : docBySlug(slug));
  const html = $derived(doc ? marked.parse(doc.markdown, { async: false }) : '');

  /** Docs are authored markdown with relative links like `/docs/x`. */
  function fixLinks(node: HTMLElement) {
    for (const a of node.querySelectorAll<HTMLAnchorElement>('a[href^="/docs/"]')) {
      a.addEventListener('click', (e) => {
        e.preventDefault();
        history.pushState({}, '', a.pathname);
        dispatchEvent(new PopStateEvent('popstate'));
      });
    }
  }
</script>

<section class="docs-page">
  {#if doc}
    <nav class="crumbs" aria-label="Breadcrumb">
      <a href="/docs">Help</a> <span>/</span> <span>{doc.title}</span>
    </nav>
    <article class="doc" use:fixLinks>
      <h1>{doc.title}</h1>
      <p class="summary">{doc.summary}</p>
      {#if doc.sections.length > 0}
        <nav class="toc" aria-label="On this page">
          <h2>On this page</h2>
          <ul>
            {#each doc.sections as s}
              <li>{s}</li>
            {/each}
          </ul>
        </nav>
      {/if}
      {@html html}
    </article>
  {:else if slug !== ''}
    <h1>Page not found</h1>
    <p>No help page called “{slug}”. Press <kbd>Ctrl</kbd>+<kbd>K</kbd> to search the help pages, or go to <a href="/docs">the help index</a>.</p>
  {:else}
    <h1>Help</h1>
    <p class="summary">
      Everything you need to use this site, in plain words. Press
      <kbd>Ctrl</kbd>+<kbd>K</kbd> (or <kbd>⌘</kbd>+<kbd>K</kbd>) anywhere to search.
    </p>
    <ul class="index">
      {#each DOCS as d (d.slug)}
        <li>
          <a href={`/docs/${d.slug}`}>
            <span class="title">{d.title}</span>
            <span class="summary">{d.summary}</span>
          </a>
        </li>
      {/each}
    </ul>
  {/if}
</section>

<style>
  .docs-page {
    max-width: 46rem;
    margin: 0 auto;
    padding: 1.5rem 1rem 4rem;
  }
  .crumbs {
    color: var(--text-2, #666);
    font-size: 0.85rem;
    margin-bottom: 1rem;
  }
  .summary {
    color: var(--text-2, #666);
    font-size: 1.05rem;
  }
  .toc {
    background: var(--surface-2, #f6f6f6);
    border: 1px solid var(--border, #ddd);
    border-radius: 8px;
    padding: 0.75rem 1rem;
    margin: 1rem 0 1.5rem;
  }
  .toc h2 {
    font-size: 0.8rem;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    margin: 0 0 0.4rem;
  }
  .toc ul {
    margin: 0;
    padding-left: 1.1rem;
  }
  .index {
    list-style: none;
    padding: 0;
    display: grid;
    gap: 0.75rem;
    margin-top: 1.25rem;
  }
  .index a {
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
    padding: 0.9rem 1rem;
    border: 1px solid var(--border, #ddd);
    border-radius: 8px;
    text-decoration: none;
    color: inherit;
  }
  .index a:hover {
    border-color: var(--accent, #0866cc);
  }
  .index .title {
    font-weight: 600;
  }
  .index .summary {
    font-size: 0.9rem;
  }
  kbd {
    border: 1px solid var(--border, #bbb);
    border-bottom-width: 2px;
    border-radius: 4px;
    padding: 0 0.35rem;
    font-size: 0.8em;
    background: var(--surface-2, #f2f2f2);
  }
</style>
