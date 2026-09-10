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
