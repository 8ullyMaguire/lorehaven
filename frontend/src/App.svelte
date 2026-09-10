<script lang="ts">
  import Button from './lib/components/Button.svelte';
  import Drawer from './lib/components/Drawer.svelte';
  import Select from './lib/components/Select.svelte';
  import Home from './routes/Home.svelte';
  import NotFound from './routes/NotFound.svelte';
  import Planned from './routes/Planned.svelte';
  import { isPlainLeftClick, matchRoute, navigate } from './lib/router';
  import {
    THEME_LABELS,
    applyTheme,
    readPreference,
    resolveTheme,
    writePreference,
    type ThemePreference,
  } from './lib/theme';

  /** Navigation (Milestone 1). Labels stay plain words, per the theme. */
  const NAV = [
    { href: '/discover', label: 'Discover', primary: true },
    { href: '/search', label: 'Search', primary: true },
    { href: '/library', label: 'Library', primary: true },
    { href: '/write', label: 'Write', primary: true },
    { href: '/community', label: 'Community', primary: false },
    { href: '/notifications', label: 'Notifications', primary: false },
    { href: '/pseud', label: 'Pseud', primary: false },
  ];

  let path = $state(window.location.pathname);
  let moreOpen = $state(false);
  let route = $derived(matchRoute(path));

  let preference = $state<ThemePreference>(readPreference(window.localStorage));

  const darkQuery =
    typeof window.matchMedia === 'function'
      ? window.matchMedia('(prefers-color-scheme: dark)')
      : null;

  // Applying the theme here keeps the switch instant; `index.html` already
  // applied the same resolution before first paint.
  $effect(() => {
    applyTheme(resolveTheme(preference, darkQuery?.matches ?? false), document.documentElement);
  });

  $effect(() => {
    const onPopState = () => {
      path = window.location.pathname;
      moreOpen = false;
    };
    window.addEventListener('popstate', onPopState);

    const onSystemChange = () => {
      if (preference === 'system') {
        applyTheme(resolveTheme('system', darkQuery?.matches ?? false), document.documentElement);
      }
    };
    darkQuery?.addEventListener('change', onSystemChange);

    return () => {
      window.removeEventListener('popstate', onPopState);
      darkQuery?.removeEventListener('change', onSystemChange);
    };
  });

  function onLinkClick(event: MouseEvent, href: string) {
    if (!isPlainLeftClick(event)) return;
    event.preventDefault();
    navigate(href);
  }

  const themeOptions = (Object.keys(THEME_LABELS) as ThemePreference[]).map((value) => ({
    value,
    label: THEME_LABELS[value],
  }));
</script>

<a class="skip-link" href="#main">Skip to content</a>

<header class="site-header">
  <div class="container bar">
    <a class="brand" href="/" onclick={(event) => onLinkClick(event, '/')}>
      <svg class="mark" viewBox="0 0 64 64" aria-hidden="true" focusable="false">
        <g fill="none" stroke="currentColor" stroke-width="3.5" stroke-linecap="round" stroke-linejoin="round">
          <path d="M32 20v28" />
          <path d="M32 20c-5-3.4-11.5-5-22-5v28c10.5 0 17 1.6 22 5" />
          <path d="M32 20c5-3.4 11.5-5 22-5v28c-10.5 0-17 1.6-22 5" />
        </g>
        <path
          d="M24 41v-8a8 8 0 0 1 16 0v8"
          fill="none"
          stroke="var(--color-accent)"
          stroke-width="3"
          stroke-linecap="round"
        />
      </svg>
      <span class="wordmark">Lorehaven</span>
    </a>

    <nav class="desktop" aria-label="Main">
      {#each NAV as item (item.href)}
        <a
          href={item.href}
          aria-current={path === item.href ? 'page' : undefined}
          onclick={(event) => onLinkClick(event, item.href)}
        >
          {item.label}
        </a>
      {/each}
    </nav>

    <div class="controls">
      <label class="visually-hidden" for="theme-select">Appearance</label>
      <select
        id="theme-select"
        bind:value={preference}
        onchange={() => writePreference(window.localStorage, preference)}
      >
        {#each themeOptions as option (option.value)}
          <option value={option.value}>{option.label}</option>
        {/each}
      </select>
    </div>
  </div>

  <!-- Mobile navigation: four destinations plus everything else behind More. -->
  <nav class="mobile" aria-label="Main">
    {#each NAV.filter((item) => item.primary) as item (item.href)}
      <a
        href={item.href}
        aria-current={path === item.href ? 'page' : undefined}
        onclick={(event) => onLinkClick(event, item.href)}
      >
        {item.label}
      </a>
    {/each}
    <Button variant="quiet" size="sm" onclick={() => (moreOpen = true)}>More</Button>
  </nav>
</header>

<main id="main" class="container" tabindex="-1">
  {#if route.id === 'home'}
    <Home />
  {:else if route.id === 'planned' && route.planned}
    <Planned route={route.planned} />
  {:else}
    <NotFound path={route.path} />
  {/if}
</main>

<footer class="site-footer">
  <div class="container">
    <div class="book-divider">Lorehaven</div>
    <p>
      A self-hosted home for fanfiction. Built in the open; the plan, the
      decision records and the verification log all live in the repository.
    </p>
  </div>
</footer>

<Drawer title="More" open={moreOpen} onclose={() => (moreOpen = false)}>
  <nav class="drawer-nav" aria-label="More destinations">
    {#each NAV as item (item.href)}
      <a href={item.href} onclick={(event) => onLinkClick(event, item.href)}>{item.label}</a>
    {/each}
  </nav>
  <label class="drawer-label" for="theme-select-mobile">Appearance</label>
  <Select
    id="theme-select-mobile"
    label="Appearance"
    options={themeOptions}
    bind:value={preference}
    onchange={() => writePreference(window.localStorage, preference)}
  />
</Drawer>

<style>
  .skip-link {
    position: absolute;
    left: var(--space-4);
    top: -3rem;
    z-index: 60;
    background: var(--color-primary);
    color: var(--color-primary-contrast);
    padding: var(--space-2) var(--space-4);
    border-radius: var(--radius-md);
    transition: top var(--duration-fast) ease;
  }

  .skip-link:focus {
    top: var(--space-3);
  }

  .site-header {
    background: var(--color-surface);
    border-bottom: var(--border-width) solid var(--color-border);
    position: sticky;
    top: 0;
    z-index: 20;
  }

  .bar {
    display: flex;
    align-items: center;
    gap: var(--space-5);
    min-height: 4rem;
  }

  .brand {
    display: inline-flex;
    align-items: center;
    gap: var(--space-3);
    text-decoration: none;
    color: var(--color-text);
  }

  .mark {
    width: 2rem;
    height: 2rem;
    color: var(--color-primary);
  }

  .wordmark {
    font-family: var(--font-heading);
    font-size: var(--text-xl);
    font-weight: 600;
    letter-spacing: -0.01em;
  }

  .desktop {
    display: none;
    gap: var(--space-5);
    margin-left: var(--space-5);
    flex: 1;
  }

  .desktop a {
    color: var(--color-muted);
    text-decoration: none;
    font-weight: 600;
    padding: var(--space-2) 0;
    border-bottom: 2px solid transparent;
  }

  .desktop a:hover {
    color: var(--color-text);
  }

  .desktop a[aria-current='page'] {
    color: var(--color-text);
    border-bottom-color: var(--color-accent);
  }

  .controls {
    margin-left: auto;
  }

  .controls select {
    font: inherit;
    font-size: var(--text-sm);
    color: var(--color-text);
    background: var(--color-bg);
    border: var(--border-width) solid var(--color-border-strong);
    border-radius: var(--radius-md);
    padding: var(--space-2) var(--space-3);
    min-height: 2.5rem;
  }

  .mobile {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    overflow-x: auto;
    padding: 0 var(--space-4) var(--space-3);
  }

  .mobile a {
    color: var(--color-muted);
    text-decoration: none;
    font-weight: 600;
    font-size: var(--text-sm);
    white-space: nowrap;
  }

  .mobile a[aria-current='page'] {
    color: var(--color-text);
  }

  main {
    padding: var(--space-5) var(--space-4) var(--space-8);
    min-height: 60vh;
  }

  .site-footer {
    border-top: var(--border-width) solid var(--color-border);
    background: var(--color-surface);
    padding: var(--space-5) 0 var(--space-7);
    color: var(--color-muted);
    font-size: var(--text-sm);
  }

  .site-footer p {
    max-width: 52ch;
  }

  .drawer-nav {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin-bottom: var(--space-5);
  }

  .drawer-nav a {
    padding: var(--space-2) 0;
    text-decoration: none;
    font-weight: 600;
  }

  .drawer-label {
    font-size: var(--text-sm);
    color: var(--color-muted);
  }

  /* Desktop layout: full navigation, no mobile strip. */
  @media (min-width: 52rem) {
    .desktop {
      display: flex;
    }

    .mobile {
      display: none;
    }
  }
</style>
