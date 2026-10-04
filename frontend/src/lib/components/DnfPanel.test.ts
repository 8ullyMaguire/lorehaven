import { describe, expect, it, vi, beforeEach, type Mock } from 'vitest';
import { render, screen, waitFor, fireEvent } from '@testing-library/svelte';
import { readFileSync } from 'node:fs';
import DnfPanel, { type DnfPanelProps } from './DnfPanel.svelte';
import { DNF_REASONS, type DnfRecord, type DnfReasonCount } from '../api';

/**
 * Read a file from the repository root.
 *
 * `process.cwd()` + `'../'` is the convention `src/lib/prepaint.test.ts` already uses for
 * exactly this — reading a file outside `src/` to assert against a real artefact. The two
 * rejected alternatives are worth recording:
 *
 *  - `node:path` is not imported, because this project's tsconfig excludes Node types
 *    (`Cannot find module 'node:path'`), and adding a dependency so a test can read a
 *    Rust file would be the wrong trade;
 *  - `new URL('../../../../', import.meta.url)` fails at runtime with `The URL must be of
 *    scheme file`, because vitest's jsdom environment does not give `import.meta.url` a
 *    `file:` scheme. It type-checks and then dies at import time, which is the worst
 *    shape: the whole file reports "no tests" rather than one failing test.
 */
const readRepoFile = (relative: string): string =>
  readFileSync(`${process.cwd()}/../${relative}`, 'utf8');

/**
 * A reader's own mark, as the server returns it.
 *
 * `account_id`/`pseud_id` are included even though the panel ignores them: a fixture that
 * omits a field cannot catch a component that starts reading one, and this is the shape
 * `DnfResponse` actually serialises.
 */
function record(over: Partial<DnfRecord> = {}): DnfRecord {
  return {
    id: 'dnf-1',
    account_id: 'acct-1',
    pseud_id: 'pseud-1',
    work_id: 'work-1',
    reason: 'slow_pacing',
    note: 'the middle dragged',
    is_public: false,
    created_at: '2026-10-01T10:00:00Z',
    updated_at: '2026-10-01T10:00:00Z',
    ...over,
  };
}

/**
 * The four injectable clients, with a working default for each.
 *
 * Typed as `Partial<DnfPanelProps>` rather than `Record<string, unknown>` and
 * then `as never`: the `as never` silenced the compiler on the spread while making every
 * override unchecked, which is how a typo in a test's `loadReasons` becomes a test that
 * silently used the real client. Here an override that is not one of the four is a type
 * error instead.
 */
type PanelProps = DnfPanelProps;

function deps(over: Partial<PanelProps> = {}): PanelProps {
  return {
    workId: 'work-1',
    loadMine: vi.fn(async () => null as DnfRecord | null),
    loadReasons: vi.fn(async () => [] as DnfReasonCount[]),
    save: vi.fn(
      async (
        _workId: string,
        reason: DnfRecord['reason'],
        options?: { note?: string | null; isPublic?: boolean },
      ) => record({ reason, note: options?.note ?? null, is_public: options?.isPublic ?? false }),
    ),
    clear: vi.fn(async () => undefined),
    ...over,
  };
}

describe('DnfPanel', () => {
  beforeEach(() => vi.restoreAllMocks());

  describe('signed out', () => {
    it('renders nothing at all', () => {
      render(DnfPanel, { props: { ...deps(), signedIn: false } });
      expect(screen.queryByRole('heading', { name: /did not finish/i })).toBeNull();
      expect(screen.queryByText(/mark as dnf/i)).toBeNull();
    });

    it('does not ask the server anything', () => {
      // A DNF row belongs to a pseud, so a signed-out reader has no row to write. A fetch
      // here would 401 on every work page for every anonymous visitor.
      const d = deps();
      render(DnfPanel, { props: { ...d, signedIn: false } });
      expect(d.loadMine as Mock).not.toHaveBeenCalled();
    });
  });

  describe('the offer', () => {
    it('offers to mark a work the reader has not marked', async () => {
      render(DnfPanel, { props: { ...deps(), signedIn: true } });
      await waitFor(() => expect(screen.getByTestId('dnf-open')).toBeTruthy());
      expect(screen.getByText(/keeps it out of your recommendations/i)).toBeTruthy();
    });

    it('opens a form with all six reasons', async () => {
      render(DnfPanel, { props: { ...deps(), signedIn: true } });
      await fireEvent.click(await screen.findByTestId('dnf-open'));
      // Anchored on the radio ROLE, not on a text regex. `/other/i` matches BOTH the
      // "Other" option's label and the hint on "Dropped for another reason", so a text
      // matcher is ambiguous here — and an ambiguous matcher is the same defect class as
      // the section-scoped locator that made an earlier rail test fail against correct
      // code.
      const radios = screen.getAllByRole('radio');
      expect(radios).toHaveLength(DNF_REASONS.length);
      for (const r of DNF_REASONS) {
        const radio = radios.find((el) => (el as HTMLInputElement).value === r.value);
        expect(radio, `no radio for ${r.value}`).toBeTruthy();
        expect(radio?.closest('label')?.textContent).toContain(r.label);
      }
    });
  });

  describe('saving', () => {
    it('sends the chosen reason and defaults to private', async () => {
      const d = deps();
      render(DnfPanel, { props: { ...d, signedIn: true } });
      await fireEvent.click(await screen.findByTestId('dnf-open'));
      await fireEvent.click(screen.getByLabelText(/not my taste/i));
      await fireEvent.click(screen.getByRole('button', { name: /^save$/i }));

      await waitFor(() => expect(d.save as Mock).toHaveBeenCalledOnce());
      // THE PRIVACY DEFAULT. 0073 made is_public default 0; the client must not send true
      // by omission, so this asserts the ARGUMENT rather than trusting the server.
      expect((d.save as Mock).mock.calls[0][2]).toEqual({
        note: null,
        isPublic: false,
      });
    });

    it('sends the reader\'s note verbatim and only when they wrote one', async () => {
      const d = deps();
      render(DnfPanel, { props: { ...d, signedIn: true } });
      await fireEvent.click(await screen.findByTestId('dnf-open'));
      const note = screen.getByLabelText(/note to yourself/i);
      await fireEvent.input(note, { target: { value: '  dragged in the middle  ' } });
      await fireEvent.click(screen.getByRole('button', { name: /^save$/i }));

      await waitFor(() => expect(d.save as Mock).toHaveBeenCalled());
      // Trimmed on the way out, and `null` rather than '' when empty — an empty note and
      // no note are the same thing to the column, and '' would store a row that claims
      // the reader wrote something.
      expect((d.save as Mock).mock.calls[0][2].note).toBe(
        'dragged in the middle',
      );
    });

    it('shows the reader their own mark afterwards, in reader words', async () => {
      const d = deps({
        save: vi.fn(async (_w: string, r: DnfRecord['reason']) =>
          record({ reason: r, is_public: true }),
        ),
      });
      render(DnfPanel, { props: { ...d, signedIn: true } });
      await fireEvent.click(await screen.findByTestId('dnf-open'));
      await fireEvent.click(screen.getByLabelText(/author abandoned it/i));
      await fireEvent.click(screen.getByRole('button', { name: /^save$/i }));

      // The stored value is `abandoned_by_author`; the reader must never see it.
      await waitFor(() => expect(screen.getByTestId('dnf-current')).toBeTruthy());
      const shown = screen.getByTestId('dnf-current').textContent ?? '';
      expect(shown).toContain('Author abandoned it');
      expect(shown).not.toContain('abandoned_by_author');
      expect(shown).toContain('shared with the author');
    });

    it('keeps a private mark labelled private', async () => {
      render(DnfPanel, { props: { ...deps(), signedIn: true } });
      await fireEvent.click(await screen.findByTestId('dnf-open'));
      await fireEvent.click(screen.getByRole('button', { name: /^save$/i }));
      await waitFor(() => expect(screen.getByTestId('dnf-current')).toBeTruthy());
      expect(screen.getByTestId('dnf-current').textContent).toContain('private');
      expect(screen.getByTestId('dnf-current').textContent).not.toContain('shared with the author');
    });

    it('does not double-submit while a save is in flight', async () => {
      let release: (v: DnfRecord) => void = () => {};
      const d = deps({
        save: vi.fn(
          () =>
            new Promise<DnfRecord>((res) => {
              release = res;
            }),
        ),
      });
      render(DnfPanel, { props: { ...d, signedIn: true } });
      await fireEvent.click(await screen.findByTestId('dnf-open'));
      const save = screen.getByRole('button', { name: /^save$/i });
      await fireEvent.click(save);
      await fireEvent.click(save);
      await fireEvent.click(save);
      expect(d.save as Mock).toHaveBeenCalledOnce();
      release(record());
      await waitFor(() => expect(screen.getByTestId('dnf-current')).toBeTruthy());
    });

    it('reports a refusal instead of claiming success', async () => {
      const d = deps({
        save: vi.fn(async () => {
          throw new Error('That reason is not one this archive knows.');
        }),
      });
      render(DnfPanel, { props: { ...d, signedIn: true } });
      await fireEvent.click(await screen.findByTestId('dnf-open'));
      await fireEvent.click(screen.getByRole('button', { name: /^save$/i }));
      await waitFor(() => expect(screen.getByTestId('dnf-message')).toBeTruthy());
      expect(screen.getByTestId('dnf-message').textContent).toContain('not one this archive knows');
      // And it must not have flipped into the marked state.
      expect(screen.queryByTestId('dnf-current')).toBeNull();
    });
  });

  describe('the existing mark', () => {
    it('shows the reason the reader chose, not the first one in the enum', async () => {
      // loadMine returns slow_pacing. A panel that rendered `mine.reason` through its own
      // default would show "Not my taste" — a wrong answer the reader cannot detect.
      const d = deps({ loadMine: vi.fn(async (_workId: string, _signal?: AbortSignal) =>
          record({ reason: 'triggering' }),
        ) });
      render(DnfPanel, { props: { ...d, signedIn: true } });
      await waitFor(() => expect(screen.getByTestId('dnf-current')).toBeTruthy());
      expect(screen.getByTestId('dnf-current').textContent).toContain('Triggering content');
    });

    it('clears the mark and says so', async () => {
      const d = deps({ loadMine: vi.fn(async (_workId: string, _signal?: AbortSignal) => record()) });
      render(DnfPanel, { props: { ...d, signedIn: true } });
      await waitFor(() => expect(screen.getByTestId('dnf-current')).toBeTruthy());
      await fireEvent.click(screen.getByRole('button', { name: /clear this mark/i }));
      await waitFor(() => expect(d.clear as Mock).toHaveBeenCalledOnce());
      await waitFor(() => expect(screen.getByTestId('dnf-open')).toBeTruthy());
      expect(screen.getByTestId('dnf-message').textContent).toContain('Cleared');
    });

    it('survives a load failure, says why, and still offers the button', async () => {
      // This test found a REAL defect: `load()` had no catch, so one network blip on any
      // work page raised an unhandled rejection — which fails the whole vitest run, not
      // this test. The panel now catches, tells the reader, and still lets them mark.
      const d = deps({
        loadMine: vi.fn(async (_workId: string, _signal?: AbortSignal) => {
          throw new Error('Could not reach Lorehaven.');
        }),
        loadReasons: vi.fn(async (_workId: string, _signal?: AbortSignal) => []),
      });
      render(DnfPanel, { props: { ...d, signedIn: true } });
      await waitFor(() => expect(screen.getByTestId('dnf-message')).toBeTruthy());
      expect(screen.getByTestId('dnf-message').textContent).toContain('Could not reach Lorehaven');
      await waitFor(() => expect(screen.getByTestId('dnf-open')).toBeTruthy());
    });

    it('still renders when only the AGGREGATE fails', async () => {
      // The aggregate is the author's disclosure, not the reader's own row. Failing to
      // load it must not cost the reader the ability to record their own mark.
      const d = deps({
        loadReasons: vi.fn(async (_workId: string, _signal?: AbortSignal) => {
          throw new Error('Could not reach Lorehaven.');
        }),
      });
      render(DnfPanel, { props: { ...d, signedIn: true } });
      await waitFor(() => expect(screen.getByTestId('dnf-open')).toBeTruthy());
      expect(screen.getByTestId('dnf-message').textContent).toContain('Could not reach Lorehaven');
    });
  });

  describe('the aggregate after a write', () => {
    it('re-reads the counts after SAVING, not just at mount', async () => {
      // A REAL defect, found by the Playwright journey that asserts "1 reader stopped
      // here": the panel rendered "shared with the author" and no aggregate, because the
      // counts had been fetched once at mount — before the write that changed them. The
      // reader's own mark and the author's disclosure of it come from different queries
      // and only one was being re-read.
      const loadReasons = vi
        .fn()
        .mockResolvedValueOnce([] as DnfReasonCount[])
        .mockResolvedValueOnce([{ reason: 'slow_pacing', count: 1 }] as DnfReasonCount[]);
      render(DnfPanel, { props: { ...deps({ loadReasons }), signedIn: true } });

      await waitFor(() => expect(screen.getByTestId('dnf-open')).toBeTruthy());
      expect(screen.queryByText(/stopped here/i)).toBeNull();

      await fireEvent.click(screen.getByTestId('dnf-open'));
      await fireEvent.click(screen.getByRole('radio', { name: /too slow/i }));
      await fireEvent.click(screen.getByRole('button', { name: /^save$/i }));

      await waitFor(() => expect(screen.getByText(/1 reader stopped here/i)).toBeTruthy());
      expect(loadReasons).toHaveBeenCalledTimes(2);
    });

    it('re-reads the counts after CLEARING', async () => {
      const loadReasons = vi
        .fn()
        .mockResolvedValueOnce([{ reason: 'slow_pacing', count: 1 }] as DnfReasonCount[])
        .mockResolvedValueOnce([] as DnfReasonCount[]);
      render(DnfPanel, {
        props: {
          ...deps({ loadReasons, loadMine: vi.fn(async (_workId: string, _signal?: AbortSignal) => record()) }),
          signedIn: true,
        },
      });
      await waitFor(() => expect(screen.getByTestId('dnf-current')).toBeTruthy());
      await fireEvent.click(screen.getByRole('button', { name: /clear this mark/i }));
      // A summary still counting a mark this reader just removed is the kind of small
      // wrongness that makes an honest page untrustworthy.
      await waitFor(() => expect(screen.queryByText(/stopped here/i)).toBeNull());
    });

    it('does NOT replace the save receipt with a read failure', async () => {
      // The mark WAS written. Announcing "could not load" here would make a successful
      // save look lost, which is worse than a stale count a reload fixes.
      const loadReasons = vi
        .fn()
        .mockResolvedValueOnce([] as DnfReasonCount[])
        .mockRejectedValueOnce(new Error('Could not reach Lorehaven.'));
      render(DnfPanel, { props: { ...deps({ loadReasons }), signedIn: true } });
      await waitFor(() => expect(screen.getByTestId('dnf-open')).toBeTruthy());
      await fireEvent.click(screen.getByTestId('dnf-open'));
      await fireEvent.click(screen.getByRole('button', { name: /^save$/i }));

      await waitFor(() => expect(screen.getByTestId('dnf-current')).toBeTruthy());
      expect(screen.getByTestId('dnf-message').textContent).toContain('Saved');
      expect(screen.getByTestId('dnf-message').textContent).not.toContain('Could not reach');
    });

    it('tells a reader which of the two things happened, because they differ', async () => {
      const shared = deps({
        save: vi.fn(
          async (
            _workId: string,
            reason: DnfRecord['reason'],
            options?: { note?: string | null; isPublic?: boolean },
          ) => record({ reason, is_public: options?.isPublic ?? false }),
        ),
      });
      const { unmount } = render(DnfPanel, {
        props: { ...shared, signedIn: true },
      });
      await waitFor(() => expect(screen.getByTestId('dnf-open')).toBeTruthy());
      await fireEvent.click(screen.getByTestId('dnf-open'));
      await fireEvent.click(screen.getByLabelText(/share the reason with the author/i));
      await fireEvent.click(screen.getByRole('button', { name: /^save$/i }));
      await waitFor(() => expect(screen.getByTestId('dnf-message')).toBeTruthy());
      // Never a bare "Saved.": a private mark and a shared one behave differently, and
      // telling a reader their reason went to the author when it did not is the one
      // message here that cannot be taken back.
      expect(screen.getByTestId('dnf-message').textContent).toContain('author will see');
      unmount();

      render(DnfPanel, { props: { ...deps({ workId: 'work-2' }), signedIn: true } });
      await waitFor(() => expect(screen.getByTestId('dnf-open')).toBeTruthy());
      await fireEvent.click(screen.getByTestId('dnf-open'));
      await fireEvent.click(screen.getByRole('button', { name: /^save$/i }));
      await waitFor(() => expect(screen.getByTestId('dnf-message')).toBeTruthy());
      expect(screen.getByTestId('dnf-message').textContent).toContain('unless you make it public');
    });
  });

  describe("the author's aggregate", () => {
    it('is absent when nobody has shared a reason', async () => {
      render(DnfPanel, { props: { ...deps(), signedIn: true } });
      await waitFor(() => expect(screen.getByTestId('dnf-open')).toBeTruthy());
      expect(screen.queryByText(/stopped here/i)).toBeNull();
    });

    it('counts and names the reasons when there are any', async () => {
      const d = deps({
        loadReasons: vi.fn(async (_workId: string, _signal?: AbortSignal) => [
          { reason: 'slow_pacing', count: 3 } as DnfReasonCount,
          { reason: 'abandoned_by_author', count: 1 } as DnfReasonCount,
        ]),
      });
      render(DnfPanel, { props: { ...d, signedIn: true } });
      await waitFor(() => expect(screen.getByText(/4 readers stopped here/i)).toBeTruthy());
      expect(screen.getByText('Too slow').closest('li')?.textContent).toContain('3');
      expect(screen.getByText('Author abandoned it').closest('li')?.textContent).toContain('1');
    });

    it('says "reader" not "readers" for exactly one', async () => {
      // A header that always reads "readers" is wrong in the one case a reader is most
      // likely to check — their own single mark.
      const d = deps({
        loadReasons: vi.fn(async (_workId: string, _signal?: AbortSignal) => [
          { reason: 'not_my_taste', count: 1 } as DnfReasonCount,
        ]),
        loadMine: vi.fn(async (_workId: string, _signal?: AbortSignal) => null),
      });
      render(DnfPanel, { props: { ...d, signedIn: true } });
      await waitFor(() => expect(screen.getByText(/1 reader stopped here/i)).toBeTruthy());
      expect(screen.queryByText(/1 readers stopped here/i)).toBeNull();
    });
  });

  describe('the reason labels are the server labels', () => {
    it('every DNF_REASONS label matches DnfReason::label() in the domain crate', () => {
      // The six labels are duplicated in TS because the server exposes no catalogue. This
      // is what makes the duplication safe: a rename on either side turns this red
      // instead of silently changing what a reader is offered.
      const src = readRepoFile('crates/domain/src/dnf.rs');
      const body = src.slice(src.indexOf('pub fn label'), src.indexOf('impl std::fmt::Display'));
      const serverLabels = [...body.matchAll(/=> "([^"]+)"/g)].map((m) => m[1]);
      expect(serverLabels).toHaveLength(DNF_REASONS.length);
      expect(DNF_REASONS.map((r) => r.label)).toEqual(serverLabels);
    });

    it('every value matches DnfReason::from_str()', () => {
      const src = readRepoFile('crates/domain/src/dnf.rs');
      // Slice from the impl to the END of the file, not to the next impl: `FromStr` is
      // followed by a `#[cfg(test)] mod tests`, so slicing to an anchor that comes later
      // would either run off the end (indexOf -1) or swallow the test module's own
      // literals. `slice(-1)` returning "" is the silent version of this bug — the loop
      // body never runs and the test passes having asserted nothing.
      //
      // The anchor is the FULL path `std::str::FromStr`, not `FromStr for DnfReason`: a
      // bare `FromStr` also matches the `use std::str::FromStr` inside the test module,
      // which sits later in the file and would slice from the wrong place. The
      // `toBeGreaterThan(-1)` below is what caught that.
      const start = src.indexOf('impl std::str::FromStr for DnfReason');
      expect(start).toBeGreaterThan(-1);
      const body = src.slice(start);
      for (const r of DNF_REASONS) {
        expect(body, `${r.value} is not parseable by the server`).toContain(`"${r.value}" => Ok(`);
      }
    });

    it('every value matches the CHECK constraint in migration 0073', () => {
      const sql = readRepoFile('migrations/sqlite/0073_dnf_reasons.sql');
      for (const r of DNF_REASONS) {
        expect(sql).toContain(`'${r.value}'`);
      }
    });
  });
});