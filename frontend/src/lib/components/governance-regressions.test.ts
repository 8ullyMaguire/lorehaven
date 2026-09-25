// Tests for two bugs found while clearing pre-existing type errors. Both were
// invisible to `svelte-check` and to the existing suite: one silently ignored
// the user's input, the other sent a request the server always rejected.
//
// ThreadModePicker used Svelte 4's `bind={selected}`, which Svelte 5 passes
// through as an unknown DOM attribute. The radio group never reflected the
// choice, so "Set mode" stayed disabled against the old value and the form
// could not be submitted at all.
//
// ReadingSchedule called `addScheduleSection` with the wrong arity and a
// snake_case `topic_id` instead of the topic id, and never sent the required
// `position`, so the server rejected every add with a 422.
import { render, screen, waitFor, fireEvent } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import ThreadModePicker from './ThreadModePicker.svelte';
import ReadingSchedule from './ReadingSchedule.svelte';
import { setTopicMode, getSchedule, addScheduleSection } from '../api';

vi.mock('../api', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../api')>();
  return {
    ...actual,
    setTopicMode: vi.fn(async () => {}),
    getSchedule: vi.fn(async () => []),
    addScheduleSection: vi.fn(async () => {}),
  };
});

describe('ThreadModePicker — radio selection actually tracks the user', () => {
  beforeEach(() => vi.clearAllMocks());

  it('enables submit only after a different mode is picked', async () => {
    render(ThreadModePicker, { props: { topicId: 'topic-1', mode: 'plain' } });

    const submit = screen.getByRole('button', { name: /set mode/i });
    // Unchanged: nothing to save.
    expect(submit).toBeDisabled();

    // The regression: with Svelte 4 `bind=`, `selected` never moved, so this
    // click left the button disabled and the form could not be submitted.
    await fireEvent.click(screen.getByRole('radio', { name: /reading group/i }));
    await waitFor(() => expect(submit).toBeEnabled());

    await fireEvent.click(submit);
    await waitFor(() =>
      expect(vi.mocked(setTopicMode)).toHaveBeenCalledWith('topic-1', 'reading_group'),
    );
  });

  it('does not submit when the pick is unchanged', async () => {
    render(ThreadModePicker, { props: { topicId: 'topic-1', mode: 'plain' } });
    const submit = screen.getByRole('button', { name: /set mode/i });
    expect(submit).toBeDisabled();
    expect(vi.mocked(setTopicMode)).not.toHaveBeenCalled();
  });

  it('tells the parent to refetch so the server value replaces the guess', async () => {
    const onsaved = vi.fn();
    render(ThreadModePicker, { props: { topicId: 'topic-1', mode: 'plain', onsaved } });

    await fireEvent.click(screen.getByRole('radio', { name: /wiki pin/i }));
    await fireEvent.click(screen.getByRole('button', { name: /set mode/i }));

    // The component must not write to its `mode` prop — in Svelte 5 that write
    // is dropped, leaving the comparison stuck on the server's old value.
    await waitFor(() => expect(onsaved).toHaveBeenCalled());
  });
});

describe('ReadingSchedule — add-section payload matches the server', () => {
  beforeEach(() => vi.clearAllMocks());

  it('sends topicId position and the server field names', async () => {
    render(ReadingSchedule, { props: { topicId: 'topic-9', isModerator: true } });

    const title = screen.getByPlaceholderText(/section title/i);
    await fireEvent.input(title, { target: { value: 'Book One' } });
    await fireEvent.click(screen.getByRole('button', { name: /add/i }));

    // The regression: one argument with `topic_id` and no `position`, which the
    // server rejected outright.
    await waitFor(() =>
      expect(vi.mocked(addScheduleSection)).toHaveBeenCalledWith('topic-9', {
        position: 1,
        title: 'Book One',
        chapter_start: 1,
        chapter_end: 1,
        unlocks_at: '',
      }),
    );
  });

  it('renders the sections the server returns', async () => {
    vi.mocked(getSchedule).mockResolvedValue([
      {
        position: 1,
        title: 'Book One',
        chapter_start: 1,
        chapter_end: 5,
        unlocks_at: '2026-01-01T00:00:00Z',
        created_at: '2026-01-01T00:00:00Z',
      },
    ]);
    render(ReadingSchedule, { props: { topicId: 'topic-9', isModerator: true } });
    await waitFor(() => expect(screen.getByText('Book One')).toBeInTheDocument());
  });
});
