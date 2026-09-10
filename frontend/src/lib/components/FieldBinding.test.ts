import { fireEvent, render, screen } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';
import Harness from './FieldBinding.test.svelte';

/**
 * `bind:value` on the field primitives must actually reach the parent.
 *
 * This was a real defect: the components spread their props onto the native
 * element, so `bind:value` compiled cleanly and did nothing. Every Milestone 3
 * form submitted empty while looking perfectly filled in, which is exactly the
 * kind of bug a screenshot cannot show and a unit test of the API never touches.
 */

describe('field primitives and bind:value', () => {
  it('propagates typing in a TextField to the bound value', async () => {
    const onvalues = vi.fn();
    render(Harness, { props: { onvalues } });

    const input = screen.getByLabelText('Text');
    await fireEvent.input(input, { target: { value: 'The Lantern Road' } });

    expect(onvalues).toHaveBeenLastCalledWith(
      expect.objectContaining({ text: 'The Lantern Road' }),
    );
  });

  it('propagates typing in a Textarea to the bound value', async () => {
    const onvalues = vi.fn();
    render(Harness, { props: { onvalues } });

    const area = screen.getByLabelText('Area');
    await fireEvent.input(area, { target: { value: 'A summary.' } });

    expect(onvalues).toHaveBeenLastCalledWith(expect.objectContaining({ area: 'A summary.' }));
  });

  it('propagates a selection in a Select to the bound value', async () => {
    const onvalues = vi.fn();
    render(Harness, { props: { onvalues } });

    const select = screen.getByLabelText('Choice');
    await fireEvent.change(select, { target: { value: 'b' } });

    expect(onvalues).toHaveBeenLastCalledWith(expect.objectContaining({ choice: 'b' }));
  });

  it('still calls an explicit handler passed alongside the binding', async () => {
    // The shell's appearance selector passes `value` plus `onchange`; a
    // component that swallowed either would break it.
    const onselect = vi.fn();
    render(Harness, { props: { onvalues: vi.fn(), onselect } });

    const select = screen.getByLabelText('Choice');
    await fireEvent.change(select, { target: { value: 'b' } });

    expect(onselect).toHaveBeenCalledWith('b');
  });
});
