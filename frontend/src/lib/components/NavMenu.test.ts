/**
 * NavMenu: the keyboard rules, tested.
 *
 * A disclosure menu's keyboard behaviour is invisible in a screenshot and
 * absent from most component tests, so it is exactly the thing that regresses
 * quietly. Each test below pins one rule from the component's own contract.
 *
 * The DOM is asserted, not internal state, so the tests survive a refactor of
 * how the menu is written as long as it behaves.
 */
import { render, screen, fireEvent } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';
import NavMenu from './NavMenu.svelte';

const ITEMS = [
  { href: '/discover', label: 'Discover' },
  { href: '/search', label: 'Search' },
  { href: '/media', label: 'Media' },
];

function setup(props: Partial<{ current: boolean }> = {}) {
  const onnavigate = vi.fn();
  render(NavMenu, {
    props: { label: 'Read', items: ITEMS, onnavigate, ...props },
  });
  return { onnavigate, trigger: screen.getByTestId('menu-trigger') };
}

describe('NavMenu', () => {
  it('starts closed and says so on the trigger', () => {
    const { trigger } = setup();
    expect(trigger).toHaveAttribute('aria-expanded', 'false');
    expect(screen.queryByTestId('menu-panel')).not.toBeInTheDocument();
  });

  it('opens on click and lists every destination', async () => {
    const { trigger } = setup();
    await fireEvent.click(trigger);

    expect(trigger).toHaveAttribute('aria-expanded', 'true');
    const panel = screen.getByTestId('menu-panel');
    expect(panel).toBeInTheDocument();
    for (const item of ITEMS) {
      expect(screen.getByRole('link', { name: item.label })).toHaveAttribute(
        'href',
        item.href,
      );
    }
  });

  /**
   * The rule that a `<details>` element would not have given us, and the one
   * whose absence strands keyboard users mid-page.
   */
  it('closes on Escape AND returns focus to the trigger', async () => {
    const { trigger } = setup();
    await fireEvent.click(trigger);
    const link = screen.getByRole('link', { name: 'Search' });
    link.focus();

    await fireEvent.keyDown(trigger.parentElement!, { key: 'Escape' });

    expect(screen.queryByTestId('menu-panel')).not.toBeInTheDocument();
    expect(document.activeElement).toBe(trigger);
  });

  it('moves focus down with ArrowDown and wraps at the end', async () => {
    const { trigger } = setup();
    await fireEvent.click(trigger);
    // Opening focuses the first item.
    expect(document.activeElement).toBe(screen.getByRole('link', { name: 'Discover' }));

    const menu = trigger.parentElement!;
    await fireEvent.keyDown(menu, { key: 'ArrowDown' });
    expect(document.activeElement).toBe(screen.getByRole('link', { name: 'Search' }));

    await fireEvent.keyDown(menu, { key: 'ArrowDown' });
    expect(document.activeElement).toBe(screen.getByRole('link', { name: 'Media' }));

    // Past the last item: wrap to the first.
    await fireEvent.keyDown(menu, { key: 'ArrowDown' });
    expect(document.activeElement).toBe(screen.getByRole('link', { name: 'Discover' }));
  });

  it('moves focus up with ArrowUp and wraps at the start', async () => {
    const { trigger } = setup();
    await fireEvent.click(trigger);
    const menu = trigger.parentElement!;

    expect(document.activeElement).toBe(screen.getByRole('link', { name: 'Discover' }));
    await fireEvent.keyDown(menu, { key: 'ArrowUp' });
    expect(document.activeElement).toBe(screen.getByRole('link', { name: 'Media' }));
  });

  it('jumps to the first and last item with Home and End', async () => {
    const { trigger } = setup();
    await fireEvent.click(trigger);
    const menu = trigger.parentElement!;

    await fireEvent.keyDown(menu, { key: 'End' });
    expect(document.activeElement).toBe(screen.getByRole('link', { name: 'Media' }));
    await fireEvent.keyDown(menu, { key: 'Home' });
    expect(document.activeElement).toBe(screen.getByRole('link', { name: 'Discover' }));
  });

  it('opens with ArrowDown from the closed state, without a click', async () => {
    const { trigger } = setup();
    await fireEvent.keyDown(trigger.parentElement!, { key: 'ArrowDown' });
    expect(screen.getByTestId('menu-panel')).toBeInTheDocument();
  });

  /**
   * Tab must close the menu and must NOT pull focus back to the trigger.
   * Yanking focus backwards fights the direction the reader chose, and is the
   * second-most-common way a header menu becomes unusable.
   */
  it('closes on Tab without stealing focus back to the trigger', async () => {
    const { trigger } = setup();
    await fireEvent.click(trigger);
    screen.getByRole('link', { name: 'Search' }).focus();

    await fireEvent.keyDown(trigger.parentElement!, { key: 'Tab' });

    expect(screen.queryByTestId('menu-panel')).not.toBeInTheDocument();
    expect(document.activeElement).not.toBe(trigger);
  });

  it('navigates on click and closes', async () => {
    const { onnavigate, trigger } = setup();
    await fireEvent.click(trigger);
    await fireEvent.click(screen.getByRole('link', { name: 'Media' }));

    expect(onnavigate).toHaveBeenCalledWith('/media', expect.anything());
    expect(screen.queryByTestId('menu-panel')).not.toBeInTheDocument();
  });

  /**
   * A cmd/ctrl-click opens in a new tab, and the reader is still looking at
   * this page — so the menu must stay open. Closing it here would make the
   * link feel broken in a way that is hard to report.
   */
  it('stays open on a cmd-click, because no navigation happened here', async () => {
    const { onnavigate, trigger } = setup();
    await fireEvent.click(trigger);
    await fireEvent.click(screen.getByRole('link', { name: 'Search' }), {
      metaKey: true,
    });

    expect(onnavigate).toHaveBeenCalled();
    expect(screen.getByTestId('menu-panel')).toBeInTheDocument();
  });

  it('marks itself current when a destination in the group is active', async () => {
    const { trigger } = setup({ current: true });
    // The current group gets a dot, so "you are here" is visible on the trigger
    // even though no link inside the panel is the page you are on.
    expect(trigger.querySelector('.pip')).toBeInTheDocument();
  });

  it('has no current marker when nothing in the group is active', () => {
    const { trigger } = setup({ current: false });
    expect(trigger.querySelector('.pip')).not.toBeInTheDocument();
  });
});