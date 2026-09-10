import { fireEvent, render, screen } from '@testing-library/svelte';
import { createRawSnippet } from 'svelte';
import { describe, expect, it, vi } from 'vitest';
import Dialog from './Dialog.svelte';

/**
 * Dialog behaviour is an explicit Milestone 1 acceptance criterion: focus is
 * trapped while open and restored when it closes, and Escape closes it.
 */

const children = createRawSnippet(() => ({
  // `createRawSnippet` renders a single element, so the buttons are wrapped.
  render: () =>
    '<div><button type="button">Confirm</button><button type="button">Cancel</button></div>',
}));

describe('Dialog', () => {
  it('renders nothing while closed', () => {
    render(Dialog, { props: { open: false, title: 'Delete draft' } });
    expect(screen.queryByRole('dialog')).toBeNull();
  });

  it('announces itself as a modal dialog with an accessible name', () => {
    render(Dialog, {
      props: { open: true, title: 'Delete draft', description: 'This cannot be undone.' },
    });

    const dialog = screen.getByRole('dialog');
    expect(dialog).toHaveAttribute('aria-modal', 'true');
    expect(dialog).toHaveAccessibleName('Delete draft');
    expect(dialog).toHaveAccessibleDescription('This cannot be undone.');
  });

  it('moves focus inside when it opens', async () => {
    render(Dialog, { props: { open: true, title: 'Delete draft', children } });
    const dialog = screen.getByRole('dialog');

    // The focus move is deferred to a frame so the panel exists first.
    await new Promise((resolve) => requestAnimationFrame(resolve));
    expect(dialog.contains(document.activeElement)).toBe(true);
  });

  it('calls onclose for Escape', async () => {
    const onclose = vi.fn();
    render(Dialog, { props: { open: true, title: 'Delete draft', onclose, children } });

    await fireEvent.keyDown(screen.getByRole('dialog'), { key: 'Escape' });
    expect(onclose).toHaveBeenCalledTimes(1);
  });

  it('calls onclose from the labelled close control, which is not the only way out', async () => {
    const onclose = vi.fn();
    render(Dialog, { props: { open: true, title: 'Delete draft', onclose, children } });

    await fireEvent.click(screen.getByRole('button', { name: 'Close' }));
    expect(onclose).toHaveBeenCalledTimes(1);
  });

  it('does not close when unrelated keys are pressed', async () => {
    const onclose = vi.fn();
    render(Dialog, { props: { open: true, title: 'Delete draft', onclose, children } });

    await fireEvent.keyDown(screen.getByRole('dialog'), { key: 'a' });
    expect(onclose).not.toHaveBeenCalled();
  });
});
