<script lang="ts">
  import Skeleton from '../lib/components/Skeleton.svelte';
  import ContinueReadingBanner from '../lib/components/ContinueReadingBanner.svelte';
  import ErrorSummary from '../lib/components/ErrorSummary.svelte';
  import { handleLinkClick } from '../lib/router';
  import { session } from '../lib/session.svelte';
  import {
    fetchInstanceMeta,
    fetchReadiness,
    type InstanceMeta,
    type ReadinessReport,
  } from '../lib/api';

  let meta = $state<InstanceMeta | null>(null);
  let readiness = $state<ReadinessReport | null>(null);
  let error = $state<unknown>(null);
  let loading = $state(true);

  /**
   * Both calls are real. Nothing on this page is placeholder text dressed up as
   * data: if the server cannot answer, the page says so (spec §1.1).
   */
  async function load() {
    loading = true;
    error = null;
    try {
      const [metaResult, readinessResult] = await Promise.all([
        fetchInstanceMeta(),
        fetchReadiness(),
      ]);
      meta = metaResult;
      readiness = readinessResult;
    } catch (failure) {
      error = failure;
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    void load();
  });

  /**
   * What this place is FOR, in the reader's words, each one a real link to a
   * page that exists.
   *
   * This list was previously three milestones described as "built so far",
   * which is a changelog wearing a welcome mat's clothes. A visitor's first
   * question is not which milestone this is; it is what they can do here.
   */
  const FEATURES = [
    {
      title: 'Read without an account',
      body: 'No sign-up to start. Rating gates, age policy and everything else are decided by the server, so what you can see is never a guess in the browser.',
      href: '/discover',
      cta: 'Discover something',
    },
    {
      title: 'Write without a queue',
      body: 'Draft, revise and publish on your own schedule. Chapters are versioned and restorable, so an edit is never a leap of faith.',
      href: '/write',
      cta: 'Start writing',
    },
    {
      title: 'Bring the library you have',
      body: 'Import what you already read and keep a local copy. Exports hand the whole archive back to you whenever you want it.',
      href: '/import',
      cta: 'Import a work',
    },
    {
      title: 'Find the half-remembered one',
      body: 'Search by author, fandom, tag or rating. Media references cover the books and shows the story borrows from.',
      href: '/search',
      cta: 'Search the catalog',
    },
  ];

  /**
   * Four ways in, aimed at the reader rather than at the schema. These are the
   * same four the header offers, for the same reason: if the header and the
   * landing page disagree about what this place is, one of them is wrong.
   */
  const PATHS = [
    { label: 'Discover', detail: 'Works the community is reading', href: '/discover' },
    { label: 'Search', detail: 'By author, fandom, tag or rating', href: '/search' },
    { label: 'Library', detail: 'What you are reading and what is finished', href: '/library' },
    { label: 'Write', detail: 'Draft, revise and publish', href: '/write' },
  ];

  /** Only the checks that are not healthy are worth the reader's attention. */
  let problems = $derived(
    readiness
      ? Object.entries(readiness.checks).filter(([, check]) => !check.ok)
      : [],
  );
  let healthy = $derived(readiness !== null && problems.length === 0);
</script>

<!--
  Item 1 of the 100-idea audit. Above the hero, not below it: the point is that a
  returning reader sees where they stopped BEFORE they start scrolling for it.

  `signedIn` is passed rather than wrapped in an `{#if}` here, so the decision to make no
  request at all lives in ONE place — the component — instead of being duplicated at every
  call site. A caller that forgets the guard still gets no request, which is the failure
  mode that matters.

  An `{#if session.isSignedIn}` wrapper was here first, directly contradicting the comment
  above it, and it made the signed-out case untestable: with the wrapper, the banner node
  is not in the DOM at all when signed out, so "no request was made" had nothing to be
  asserted about. Passing the flag keeps the node mounted and hidden, which is a state a
  test can actually pin.
-->
<ContinueReadingBanner signedIn={session.isSignedIn} />

<section class="hero">
  <p class="eyebrow">A self-hosted home for fanfiction</p>
  <h1>Read, write, and keep what you love.</h1>
  <p class="lede">
    A place to read without an account, write without a publishing queue, import the
    library you already have, and take it offline when you want it. Everything you
    put here stays yours — export it whenever you like.
  </p>

  <div class="cta-row">
    <a
      class="cta primary"
      href={session.isSignedIn ? '/discover' : '/register'}
      onclick={(event) => handleLinkClick(event, session.isSignedIn ? '/discover' : '/register')}
    >
      {session.isSignedIn ? 'Open your feed' : 'Create an account'}
    </a>
    <a
      class="cta secondary"
      href="/discover"
      onclick={(event) => handleLinkClick(event, '/discover')}
    >
      Browse without signing in
    </a>
  </div>

  {#if !session.isSignedIn}
    <p class="fineprint">
      Registration is {meta?.policy.registration_open ? 'open' : 'closed on this instance'}.
      {#if meta?.policy.registration_open === false}
        You can still read everything that does not require an account.
      {/if}
    </p>
  {/if}
</section>

<section class="paths" aria-labelledby="paths-heading">
  <h2 id="paths-heading">Four ways in</h2>
  <ul class="path-grid">
    {#each PATHS as path (path.href)}
      <li>
        <a href={path.href} onclick={(event) => handleLinkClick(event, path.href)}>
          <span class="path-label">{path.label}</span>
          <span class="path-detail">{path.detail}</span>
        </a>
      </li>
    {/each}
  </ul>
</section>

<section class="features" aria-labelledby="features-heading">
  <h2 id="features-heading">What you can do here</h2>
  <ul class="feature-grid">
    {#each FEATURES as feature (feature.href)}
      <li class="feature">
        <h3>{feature.title}</h3>
        <p>{feature.body}</p>
        <a
          class="feature-cta"
          href={feature.href}
          onclick={(event) => handleLinkClick(event, feature.href)}
        >
          {feature.cta} <span aria-hidden="true">→</span>
        </a>
      </li>
    {/each}
  </ul>
</section>

{#if error}
  <ErrorSummary {error} onretry={load} />
{/if}

<!--
  Health and instance details, LAST and quiet.

  They were the first two panels on this page, at the same visual weight as
  everything else, so a visitor's first impression of the place was a build
  hash and a list of database connection strings. They are still here and still
  real — a self-hosted instance is exactly the sort of thing whose operator
  needs to see them — but they are a footnote, not the welcome.
-->
<section class="instance" aria-labelledby="instance-heading">
  <h2 id="instance-heading">This instance</h2>

  {#if loading && !meta}
    <Skeleton lines={3} label="Loading instance details" />
  {:else if meta}
    <p class="instance-line">
      <strong>{meta.name}</strong> · build <code>{meta.build}</code> · {meta.environment}
      · API <code>{meta.api_version}</code>
    </p>

    <!--
      Registration and anonymous reading are the only two policy values a
      visitor could act on. The rating ceilings are enforcement detail: the
      server decides eligibility, so printing "visitors may read up to PG-13"
      here tells a reader nothing they can use and invites them to work out
      whether the browser's answer is the real one.
    -->
    <ul class="policy-summary">
      <li>
        Registration
        <strong>{meta.policy.registration_open ? 'open' : 'closed'}</strong>
      </li>
      <li>
        Reading without an account
        <strong>{meta.policy.anonymous_reading ? 'available' : 'disabled'}</strong>
      </li>
      <li>
        Age and rating limits
        <strong>enforced by the server</strong>
      </li>
    </ul>

    <p class="note">
      Full policy and service health are at <a href="/docs">Help</a>. This instance
      reports{' '}
      {#if healthy}
        <strong class="ok-word">all services healthy</strong>.
      {:else if readiness}
        <strong class="bad-word">{problems.length} service check{problems.length === 1 ? '' : 's'} needing attention</strong>.
      {:else}
        its health is not currently reporting.
      {/if}
    </p>
  {:else}
    <p>Instance details are unavailable.</p>
  {/if}

  {#if problems.length > 0}
    <!--
      Unhealthy checks are expanded here rather than hidden behind a click. A
      self-hosted instance whose worker has stopped should say so on the front
      page, not require a reader to go looking.
    -->
    <ul class="checks">
      {#each problems as [name, check] (name)}
        <li>
          <span class="dot" aria-hidden="true"></span>
          <span class="check-name">{name}</span>
          <span class="check-detail">{check.detail}</span>
          {#if check.remedy}
            <span class="remedy">{check.remedy}</span>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
</section>

<style>
  .hero {
    padding: var(--space-8) 0 var(--space-7);
    border-bottom: var(--border-width) solid var(--color-border);
    margin-bottom: var(--space-7);
  }

  .eyebrow {
    font-family: var(--font-interface);
    font-size: var(--text-sm);
    text-transform: uppercase;
    letter-spacing: 0.1em;
    color: var(--color-accent);
    margin: 0 0 var(--space-3);
  }

  .hero h1 {
    max-width: 22ch;
    font-size: var(--text-3xl);
    line-height: 1.1;
    margin-bottom: var(--space-4);
  }

  .lede {
    font-size: var(--text-lg);
    color: var(--color-muted);
    max-width: 52ch;
    margin-bottom: var(--space-6);
  }

  .cta-row {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-3);
  }

  .cta {
    display: inline-flex;
    align-items: center;
    padding: var(--space-3) var(--space-5);
    border-radius: var(--radius-md);
    font-weight: 600;
    text-decoration: none;
    border: var(--border-width) solid transparent;
  }

  .cta.primary {
    background: var(--color-primary);
    color: var(--color-primary-contrast);
  }

  .cta.primary:hover {
    background: var(--color-primary-hover);
  }

  .cta.secondary {
    border-color: var(--color-border-strong);
    color: var(--color-text);
  }

  .cta.secondary:hover {
    background: var(--color-surface);
  }

  .cta:focus-visible {
    outline: 2px solid var(--color-focus);
    outline-offset: 2px;
  }

  .fineprint {
    margin: var(--space-4) 0 0;
    font-size: var(--text-sm);
    color: var(--color-muted);
  }

  h2 {
    font-size: var(--text-xl);
    margin-bottom: var(--space-4);
  }

  /* Four ways in: the first thing offered, and the thing a first-time visitor
     is actually looking for. Large targets, one line of detail each. */
  .path-grid {
    list-style: none;
    margin: 0 0 var(--space-7);
    padding: 0;
    display: grid;
    gap: var(--space-3);
    grid-template-columns: repeat(auto-fit, minmax(13rem, 1fr));
  }

  .path-grid a {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    height: 100%;
    padding: var(--space-4);
    border: var(--border-width) solid var(--color-border);
    border-radius: var(--radius-lg);
    background: var(--color-surface);
    text-decoration: none;
    color: var(--color-text);
  }

  .path-grid a:hover {
    border-color: var(--color-border-strong);
    box-shadow: var(--shadow-sm);
  }

  .path-grid a:focus-visible {
    outline: 2px solid var(--color-focus);
    outline-offset: 2px;
  }

  .path-label {
    font-weight: 700;
    font-size: var(--text-lg);
  }

  .path-detail {
    color: var(--color-muted);
    font-size: var(--text-sm);
  }

  /* What you can do here: the substance, so the cards explain rather than
     repeat a button label. */
  .feature-grid {
    list-style: none;
    margin: 0 0 var(--space-7);
    padding: 0;
    display: grid;
    gap: var(--space-5);
    grid-template-columns: 1fr;
  }

  @media (min-width: 46rem) {
    .feature-grid {
      grid-template-columns: 1fr 1fr;
    }
  }

  .feature {
    background: var(--color-surface);
    border: var(--border-width) solid var(--color-border);
    border-radius: var(--radius-lg);
    padding: var(--space-5);
  }

  .feature h3 {
    font-size: var(--text-lg);
    margin-bottom: var(--space-2);
  }

  .feature p {
    color: var(--color-muted);
    margin-bottom: var(--space-4);
  }

  .feature-cta {
    color: var(--color-accent);
    font-weight: 600;
    text-decoration: none;
  }

  .feature-cta:hover {
    text-decoration: underline;
  }

  /* The footnote. Deliberately smaller and quieter than everything above: this
     is instance furniture, and it used to be the masthead. */
  .instance {
    border-top: var(--border-width) solid var(--color-border);
    padding-top: var(--space-6);
    font-size: var(--text-sm);
  }

  .instance h2 {
    font-size: var(--text-base);
    font-family: var(--font-interface);
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--color-muted);
  }

  .instance-line {
    color: var(--color-muted);
    margin-bottom: var(--space-4);
  }

  .policy-summary {
    list-style: none;
    margin: 0 0 var(--space-4);
    padding: 0;
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2) var(--space-5);
    color: var(--color-muted);
  }

  .policy-summary strong {
    color: var(--color-text);
  }

  .note {
    color: var(--color-muted);
    max-width: 60ch;
    margin-bottom: 0;
  }

  .ok-word {
    color: var(--color-success);
  }

  .bad-word {
    color: var(--color-danger);
  }

  .checks {
    list-style: none;
    margin: var(--space-4) 0 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .checks li {
    display: grid;
    grid-template-columns: auto auto 1fr;
    gap: var(--space-3);
    align-items: baseline;
  }

  .dot {
    width: 0.5rem;
    height: 0.5rem;
    border-radius: 50%;
    background: var(--color-danger);
    align-self: center;
  }

  .check-name {
    font-weight: 600;
    text-transform: capitalize;
  }

  .check-detail {
    color: var(--color-muted);
    /* Readiness details name a connection: a URL with no spaces in it. Without
       this the detail sets the panel's minimum width and pushes the page
       sideways at 320 CSS pixels. */
    overflow-wrap: anywhere;
  }

  .remedy {
    grid-column: 3;
    color: var(--color-danger);
  }
</style>