<!--
  A header menu: a button that reveals a list of destinations.

  This exists because the header had eighteen links in one scrolling row, and
  `.desktop` hid its own scrollbar -- so the destinations past the fold were
  invisible rather than merely cramped. Grouping them fixes the crowding; this
  component makes the grouping operable.

  ## Keyboard behaviour, which is the whole reason this is a component and not a
  `<details>` element

  A disclosure widget has obligations that `<details>` does not meet, and the
  three that matter here are:

  1. **Escape closes and returns focus to the button.** Without the focus
     return, Escape strands focus on an element that is no longer rendered and
     the next Tab lands somewhere unrelated.
  2. **Arrow keys move between items** and wrap at both ends.
  3. **Tab closes the menu** rather than walking into it and then leaving it
     open behind you. Menus that stay open behind the keyboard are the most
     common way a header becomes unusable without a mouse.

  Clicking outside closes it too, for the pointer case.

  ## Why it is not a popover API

  `popover` would give the top layer and light-dismiss nearly free, but it is not
  in the jsdom version this repository's component tests run against, so a test
  could not have asserted any of the above. Correct behaviour you cannot test is
  a behaviour that silently regresses.

  The grouping is a pure function of props, so the test file asserts the DOM
  (items, aria-expanded, focus location) rather than any internal state.
-->
<script lang="ts">
  import type { Snippet } from 'svelte';

  interface NavItem {
    href: string;
    label: string;
  }

  interface Props {
    label: string;
    items: NavItem[];
    /** True when any destination in this group is the current page. */
    current?: boolean;
    onnavigate: (href: string, event: MouseEvent) => void;
    onopen?: () => void;
    children?: Snippet;
  }

  let { label, items, current = false, onnavigate, onopen, children }: Props = $props();

  let open = $state(false);
  let button = $state<HTMLButtonElement | null>(null);
  let menu = $state<HTMLElement | null>(null);

  /**
   * Focusable descendants, in DOM order.
   *
   * The obvious visibility filter here is `el.offsetParent !== null`, and it is
   * wrong twice over: `offsetParent` is null for every element in jsdom, and it
   * is null for any `position: fixed` element in a real browser. Filtering on it
   * empties the list under test and would drop a fixed menu item in production.
   *
   * So the list is not filtered. Every link in the panel is reachable, which is
   * the honest invariant here anyway: the panel renders only while open, so
   * there is nothing hidden in it to skip.
   */
  function focusables(): HTMLElement[] {
    if (!menu) return [];
    return Array.from(
      menu.querySelectorAll<HTMLElement>('a[href], button:not([disabled])'),
    );
  }

  function close(returnFocus = true) {
    if (!open) return;
    open = false;
    if (returnFocus) button?.focus();
  }

  function toggle() {
    open = !open;
    if (open) {
      onopen?.();
      // Focus the first item so the keyboard path does not have to Tab through
      // the page to get into the menu it just opened.
      queueMicrotask(() => focusables()[0]?.focus());
    }
  }

  function onKeydown(event: KeyboardEvent) {
    if (!open) {
      // Enter and Space are the button's own; ArrowDown opens and enters, which
      // is what a menu is expected to do from the keyboard.
      if (event.key === 'ArrowDown') {
        event.preventDefault();
        open = true;
        onopen?.();
        queueMicrotask(() => focusables()[0]?.focus());
      }
      return;
    }

    switch (event.key) {
      case 'Escape':
        event.preventDefault();
        close();
        break;
      case 'ArrowDown': {
        event.preventDefault();
        const list = focusables();
        if (list.length === 0) return;
        const at = list.indexOf(document.activeElement as HTMLElement);
        list[(at + 1) % list.length]?.focus();
        break;
      }
      case 'ArrowUp': {
        event.preventDefault();
        const list = focusables();
        if (list.length === 0) return;
        const at = list.indexOf(document.activeElement as HTMLElement);
        list[(at - 1 + list.length) % list.length]?.focus();
        break;
      }
      case 'Home': {
        event.preventDefault();
        focusables()[0]?.focus();
        break;
      }
      case 'End': {
        event.preventDefault();
        focusables().at(-1)?.focus();
        break;
      }
      case 'Tab':
        // Close without stealing the focus back -- Tab is moving onward, and
        // pulling focus to the button would fight the user's own direction.
        close(false);
        break;
    }
  }

  function onFocusOut(event: FocusEvent) {
    if (!open) return;
    const next = event.relatedTarget as Node | null;
    // Focus left the whole header region: close, but do not yank it back.
    if (!next || !event.currentTarget instanceof Node || !event.currentTarget.contains(next)) {
      close(false);
    }
  }
</script>

<div class="menu" role="group" onkeydown={onKeydown} onfocusout={onFocusOut}>
  <button
    bind:this={button}
    type="button"
    class="trigger"
    class:open
    aria-expanded={open}
    aria-haspopup="true"
    data-testid="menu-trigger"
    onclick={toggle}
  >
    {label}{#if current}<span class="pip" aria-hidden="true"></span>{/if}
  </button>

  {#if open}
    <div
      bind:this={menu}
      class="panel"
      role="group"
      aria-label={label}
      data-testid="menu-panel"
    >
      {#each items as item (item.href)}
        <a
          href={item.href}
          onclick={(event) => {
            onnavigate(item.href, event);
            // Close only when the click actually navigated. `handleLinkClick`
            // ignores a cmd/ctrl-click so the link opens in a new tab, and
            // closing the menu on that would be wrong -- the reader is still
            // looking at this page.
            if (!event.metaKey && !event.ctrlKey && !event.shiftKey && event.button === 0) {
              close(false);
            }
          }}
        >
          {item.label}
        </a>
      {/each}
      {#if children}{@render children()}{/if}
    </div>
  {/if}
</div>

<!--
  Clicking anywhere else closes the menu. Bound on the document rather than on a
  backdrop element, so there is no invisible element intercepting clicks, and so
  the page behind stays usable.
-->
<svelte:window
  onclick={(event) => {
    if (!open) return;
    if (event.target instanceof Node && menu?.contains(event.target)) return;
    if (button?.contains(event.target as Node)) return;
    open = false;
  }}
/>

<style>
  .menu {
    position: relative;
  }

  .trigger {
    font: inherit;
    font-weight: 600;
    background: none;
    border: 0;
    /* A 2px transparent bottom border, matching the link treatment, so the
       trigger's box is the same height as a link beside it and the row does not
       gain a few pixels when a menu sits between two links. */
    border-bottom: 2px solid transparent;
    color: var(--color-muted);
    cursor: pointer;
    /* No horizontal padding: the row is width-capped (see App.svelte), so
       horizontal padding on a trigger is charged directly against the space the
       nine items share. Vertical padding is what gives the trigger its box height,
       and that has to match a plain link beside it. */
    padding: var(--space-2) 0;
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
  }

  .trigger:hover,
  .trigger.open {
    color: var(--color-text);
  }

  .trigger[aria-expanded='true'] {
    color: var(--color-text);
    border-bottom-color: var(--color-border-strong);
  }

  .pip {
    width: 0.4rem;
    height: 0.4rem;
    border-radius: 50%;
    background: var(--color-accent);
  }

  .panel {
    position: absolute;
    top: calc(100% + var(--space-2));
    left: 50%;
    transform: translateX(-50%);
    /* min-width, not width: a two-item group ("Read") must not be as wide as a
       five-item one, but every panel must be at least as wide as its trigger so
       the label never wraps under the caret. */
    min-width: 13rem;
    background: var(--color-surface-raised);
    border: var(--border-width) solid var(--color-border-strong);
    border-radius: var(--radius-md);
    box-shadow: var(--shadow-lg);
    padding: var(--space-2);
    display: flex;
    flex-direction: column;
    z-index: 50;
  }

  .panel a {
    color: var(--color-text);
    text-decoration: none;
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-sm);
    white-space: nowrap;
  }

  .panel a:hover {
    background: var(--color-surface);
  }

  /* The focus ring must be visible on the panel itself as well as its links,
     because ArrowDown focuses a link but Tab-out can leave the panel focused. */
  .panel:focus-within {
    outline: 2px solid var(--color-focus);
    outline-offset: 1px;
  }

  @media (prefers-reduced-motion: no-preference) {
    .panel {
      animation: menu-in var(--duration-fast) ease;
    }

    @keyframes menu-in {
      from {
        opacity: 0;
        transform: translate(-50%, calc(-1 * var(--space-2)));
      }
      to {
        opacity: 1;
        transform: translate(-50%, 0);
      }
    }
  }
</style>