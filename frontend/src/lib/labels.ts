/**
 * How the interface names values the server owns.
 *
 * This is a *label* map, never a validation table. The accepted set for each
 * privacy key comes from `/settings/privacy`'s `schema` and is rendered from
 * there; the ratings list mirrors `ContentRating` in
 * `crates/domain/src/policy.rs`, and the server still applies its own ceiling
 * to whatever is chosen, so a mistake here cannot widen anybody's access.
 *
 * An unrecognised value is shown as it arrived rather than hidden: silently
 * dropping something the server said would be worse than an ugly label.
 */

const VALUE_LABELS: Record<string, string> = {
  // Visibility.
  private: 'Only me',
  public: 'Anyone',
  // Messaging.
  nobody: 'Nobody',
  contacts_only: 'People I follow',
  anyone: 'Anyone',
  // Directory.
  listed: 'Listed',
  hidden: 'Hidden',
  // Consent toggles.
  true: 'Allowed',
  false: 'Not allowed',
};

/** A human label for a privacy value. */
export function describeValue(value: string): string {
  return VALUE_LABELS[value] ?? value;
}

/**
 * The rating ladder, lowest first.
 *
 * Mirrors `ContentRating`; see the module note on why that is acceptable here.
 */
export const RATINGS = ['general', 'teen', 'mature', 'explicit'] as const;

const RATING_LABELS: Record<string, string> = {
  general: 'General audiences',
  teen: 'Teen and up',
  mature: 'Mature',
  explicit: 'Explicit',
};

/** A human label for a content rating. */
export function describeRating(rating: string): string {
  return RATING_LABELS[rating] ?? rating;
}

/**
 * How an age band is described, in the words the person chose for themselves.
 *
 * `unknown` is offered as a real answer rather than an error state: spec §7
 * avoids collecting birth dates, and a visitor who declines to say is treated
 * as an unknown age rather than guessed at.
 */
export const AGE_BANDS = [
  { value: 'adult', label: 'I am 18 or older' },
  { value: 'minor', label: 'I am under 18' },
  { value: 'unknown', label: 'I would rather not say' },
] as const;

/** How an age state is shown on the account page. */
const AGE_STATE_LABELS: Record<string, string> = {
  unknown: 'not stated',
  declared_minor: 'stated as under 18',
  declared_adult: 'stated as 18 or older (unverified)',
  authorization_required: 'awaiting guardian authorization',
  authorized_under_policy: 'authorized under the instance policy',
  restricted: 'restricted: reading only',
};

/** A human label for an age state. */
export function describeAgeState(state: string): string {
  return AGE_STATE_LABELS[state] ?? state;
}

// ---------------------------------------------------------------------------
// Milestone 3 — the vocabulary of works
// ---------------------------------------------------------------------------

/** A work's editorial state, in the writer's words. */
const LIFECYCLE_LABELS: Record<string, string> = {
  draft: 'Draft',
  scheduled: 'Scheduled',
  published: 'Published',
  withdrawn: 'Withdrawn',
  deleted: 'Deleted',
};

/** A human label for a work's lifecycle. */
export function describeLifecycle(lifecycle: string): string {
  return LIFECYCLE_LABELS[lifecycle] ?? lifecycle;
}

/**
 * Who can reach a work.
 *
 * The unlisted wording states the consequence rather than the setting: spec §8
 * requires that "anyone with the URL can read it" is said plainly, because
 * "unlisted" reads like "private" to most people and it is not.
 */
const VISIBILITY_LABELS: Record<string, string> = {
  public: 'Listed publicly',
  unlisted: 'Unlisted — anyone with the link can read it',
  restricted: 'Signed-in readers only',
};

/** A human label for a work's visibility. */
export function describeVisibility(visibility: string): string {
  return VISIBILITY_LABELS[visibility] ?? visibility;
}

/** How finished a work is; orthogonal to whether it is published. */
const COMPLETION_LABELS: Record<string, string> = {
  in_progress: 'In progress',
  complete: 'Complete',
  hiatus: 'On hiatus',
  abandoned: 'Abandoned',
};

/** A human label for a work's completion state. */
export function describeCompletion(completion: string): string {
  return COMPLETION_LABELS[completion] ?? completion;
}

/** The roles a contributor can hold, and what each may do. */
export const CONTRIBUTOR_ROLES = [
  { value: 'coauthor', label: 'Co-author — may edit and publish' },
  { value: 'editor', label: 'Editor — may edit, may not publish' },
  { value: 'beta_reader', label: 'Beta reader — may read, may not change text' },
] as const;

/** A human label for a contributor role. */
export function describeRole(role: string): string {
  switch (role) {
    case 'owner':
      return 'Owner';
    case 'coauthor':
      return 'Co-author';
    case 'editor':
      return 'Editor';
    case 'beta_reader':
      return 'Beta reader';
    default:
      return role;
  }
}

/**
 * A byte count a reader can read.
 *
 * Binary units, because this measures files the browser is holding — and the
 * numbers are rounded to one decimal at most, since a file that is 3.7 MB is not
 * usefully "3.7276611328125 MB".
 */
export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return 'unknown size';
  if (bytes < 1024) return `${bytes} B`;
  const units = ['KB', 'MB', 'GB'];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value.toFixed(value >= 10 ? 0 : 1)} ${units[unit]}`;
}

// ---------------------------------------------------------------------------
// Work discussion modes and typed-vote reactions (spec §35.0–35.1)
// ---------------------------------------------------------------------------

/** A human label for a work discussion mode. */
export function describeDiscussionMode(mode: string): string {
  switch (mode) {
    case 'thread_only':
      return 'Reactions and linked discussion';
    case 'comments_only':
      return 'Inline comments';
    case 'both':
      return 'Reactions, comments, and linked discussion';
    default:
      return mode;
  }
}

/** A human label for a reaction vote type. */
export function describeReactionType(voteType: string): string {
  switch (voteType) {
    case 'well_written':
      return 'Well written';
    case 'insightful':
      return 'Insightful';
    case 'funny':
      return 'Funny';
    case 'interesting':
      return 'Interesting';
    case 'disagree':
      return 'Disagree';
    default:
      return voteType;
  }
}

/** A short emoji/icon glyph for a reaction vote type. */
export function reactionGlyph(voteType: string): string {
  switch (voteType) {
    case 'well_written':
      return '✍️';
    case 'insightful':
      return '💡';
    case 'funny':
      return '😂';
    case 'interesting':
      return '🤔';
    case 'disagree':
      return '👎';
    default:
      return '⭐';
  }
}
