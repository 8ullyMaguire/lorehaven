# 21. Frontend and design

**Status:** reserved
**Milestone:** M26
**Source:** the corresponding section of the Darkhold specification

## 21.1 Principles

- **Tokens are the source of truth.** `frontend/src/styles/tokens.css`; component CSS uses
  `var(--…)`.
- **The reader's appearance is independent of the site's.** A `--reader-*` scale, separate from the
  site scale, with its own presets.
- **The interface disappears while reading.** No badges, no points, no streak warnings, no
  suggestion popups over the text.
- **A signed-out visitor never sees "That did not work."** A 401 is the answer to "is this page for
  me?", and the page says so.
- **Every empty state explains itself.**
- **Accessibility is a floor:** the reader works without JavaScript, contrast ratios are checked in
  CI, and the keyboard path through the reader is complete.

## 21.2 The Darkhold identity

**The theme is reworked around the new name** — the owner's decision, and the one that costs the
most to get right.

**And it must be simpler.** The owner's ruling, 2026-10-19: *"turn lorehaven into simpler
website look like https://novellunar.com/"*. Simplicity is the requirement; the Darkhold register
is the subject matter. A dark, ornate interface is not simpler, and a simpler one is not
therefore plain.

**What "like novellunar" means, concretely** — measured from the site itself, not from a mood:

- **A flat, seven-item top nav** — Home · Browse · Latest · New · Completed · Ranking · Genres.
  One row. No mega-menu, no nested taxonomy in the header.
- **A single sidebar** — "Browse by Genre", 74 entries, with a *View all* link. The taxonomy is
  one column beside the content, not a drawer and not a tree widget.
- **A ranked list at the top** — "Weekly Trending", numbered, with the chapter count, the view
  count and a star rating on each row. A leaderboard, not a carousel.
- **A cover-card grid** — `aspect-[3/4]` covers with a gradient scrim, the metadata overlaid on
  the scrim, and a zoom on hover. The card is the unit of browsing.
- **A layered dark scale, not black** — `#1a1a1a` / `#1e1e1e` / `#252525` with
  `border-gray-700` / `border-gray-800`. Three greys, stepped. **This is the answer to the
  "the reader becomes unreadable in a dark theme" risk (R-15)**, and it is why the Dark preset's
  reading surface is a dark surface and not a paper one.
- **One sans family**, one scale, and icons used sparingly.

**And what the design must express** is the register the instance actually holds: **power,
dominance, corruption, progression, the dark side of fanfiction** — without becoming a screencap
of a grimdark game UI. The reference points are the Machiavelli shelf and the tag distribution of
§2.1, not a colour palette someone liked.

**Simplicity is a testable property, not a taste.** The pages that carry the identity are the
front page, the work page, the reader, the theme and channel pages, and the browse grid — and
**each of them is measured against the reference above**: one nav row, one sidebar, one card
grid, one ranked list, three greys. A page that needs a second navigation column to work has
failed.

**Constraints the rework inherits and must not break:**

1. **The token architecture.** One source of truth, `--reader-*` separate from the site scale,
   component CSS using `var(--…)` only. The **values** change; the architecture does not.
2. **The reader stays readable.** The single hardest constraint. A dark, high-contrast interface is
   where prose dies: the reading page needs a paper-like surface or a carefully chosen dark
   surface, and the contrast must be **measured against a named standard**, not eyeballed.

   **The standard is WCAG 2.2 AA**, and it is a gate, not an aspiration:
   - **4.5:1** minimum contrast for body text and for text in the reading surface.
   - **3:1** minimum for large text (≥ 24 px, or ≥ 18.66 px bold) and for UI components and
     their boundaries — focus rings, input borders, control edges, selected-state markers.
   - **4.5:1** for the text of every interactive element, including a link inside a body of text.
   - **The reading surface is checked separately and at its own presets.** A theme that passes AA
     on the site chrome and fails on the reading page is a failure, and the gate is per preset:
     Paper, White, Sepia, Dark and Custom.
   - **Focus is always visible** (2.4.7), **target size** is ≥ 24×24 CSS px (2.5.8), and
     **no interaction is keyboard-only-blocked** (2.1.1).
   - The **contrast check is automated** in the frontend gate: every token pair that the design
     declares as a foreground/background combination is asserted against the ratio, so a new
     preset cannot be merged without passing. A preset that cannot be checked is not merged.
3. **The three presets become the Darkhold presets**, and the site and reader presets stay
   independently selectable.
4. **543 passing frontend tests** encode the *architecture*, not the palette — but any test that
   asserts a colour, a token value or a rendered snapshot must be re-derived, and the rework must
   not be allowed to quietly weaken an assertion to make it pass.
5. **A theme rework does not re-skin the semantic tokens.** `--success`, `--warning`,
  `--content-restricted` and friends must keep their meaning; the visual weight may change, the
   signal may not.

**The work is sequenced after the backend** (§24.3) so the pages the identity depends on — the
front page and the instance theme's statement, the reader, the theme and channel pages — exist
before they are dressed.

## 21.3 Site profiles

A site profile is a TOML-selected bundle: layout template, token set, navigation structure, default
reader settings, feature toggles, and a theme identity. A skin changes **presentation only, never
policy**.

## 21.4 The gate

`cargo fmt` · `cargo clippy` (workspace, `warnings` denied) · `cargo test` on **both** engines ·
doctests **standalone** · `svelte-check` · vitest · Playwright **one suite at a time**.

---
