# Lorehaven — Theme: “The Reading Room”

## Lorehaven: a living library, not a fantasy tavern

Give Lorehaven the warmth of a beloved reading room with the clarity of a modern
writing app. The name already carries the fantasy association; the interface
doesn’t need parchment textures, ornate borders, or medieval lettering.

**Theme name: “The Reading Room.”**

### 1. Visual identity

- Warm paper backgrounds.
- Deep evergreen navigation and primary actions.
- Muted copper accents.
- Comfortable spacing and fine borders.
- Bookish headings paired with clean interface text.
- Minimal animation and no decorative clutter.

The personality should feel **welcoming, thoughtful, and quietly imaginative**—
broad enough for every fandom.

### 2. Color palette

| Role | Light theme | Dark theme |
|---|---|---|
| Page background | `#F6F3EC` — warm paper | `#151C19` — forest charcoal |
| Surface/cards | `#FFFDFA` — ivory | `#1E2923` — deep moss |
| Main text | `#202B25` — ink | `#EEEFE7` — pale ivory |
| Secondary text | `#59645C` — slate green | `#ADB9AE` — sage gray |
| Primary action | `#245C46` — evergreen | `#9BCBAA` — soft green |
| Accent | `#955333` — copper | `#D8A17B` — warm copper |
| Borders | `#D9DFD5` | `#39483E` |

Use copper sparingly for selected details, milestones, and decorative
emphasis—not every button. Validate final text, focus, and control combinations
for accessibility.

### 3. Typography

**Interface:** Source Sans 3
Readable, personable, and effective in dense search filters.

**Headings:** Source Serif 4
Adds literary character without looking theatrical.

**Reader default:** Literata
Designed for comfortable long-form reading.

Self-host the fonts, subset them, and lazy-load optional reader fonts. Offer
system fonts as a lightweight choice.

Avoid handwritten fonts in the interface. They make a library feel like a
scrapbook and are harder to read.

### 4. Logo

An **open book whose central negative space suggests an archway**: a small
visual expression of “a haven for stories.”

- Simple enough for a favicon.
- Recognizable in one color.
- No tiny book spines or elaborate scenery.
- Pair with a restrained serif wordmark: **Lorehaven**.

### 5. How the main pages should feel

**Home — a welcoming library entrance**
A prominent search field, followed by Continue Reading, library updates, and
configurable discovery sections. Logged-out visitors see a brief introduction
and immediate access to stories—not an oversized marketing banner.

**Search — a useful catalog**
Text-first result rows with clear titles, summaries, completion status, main
characters, and central relationships. Desktop filters sit beside results;
mobile filters open in a drawer. Don’t turn every tag into a brightly colored
pill.

**Reader — almost invisible interface**
Comfortable margins, excellent typography, and discreet controls. Separate
reader appearance from the site theme so someone can use a dark interface with
a warm-paper reading page.

**Writer dashboard — a calm workspace**
“Continue writing” comes first. Show drafts, chapter status, and saved-state
information clearly. Goals and achievements stay secondary.

**Community — a shared common room**
Familiar discussion layouts, readable conversations, and understated avatars.
Avoid making the forum look like a different product.

### 6. Distinctive details

Use a few recurring motifs:

- A bookmark ribbon for saved works.
- Fine book-divider lines between sections.
- A subtle arch shape in empty-state illustrations.
- Small spine-like color accents for shelves and collections.

Avoid excessive literary naming. Navigation should still say **Search, Library,
Write, Community**—not “The Observatory” or “The Scriptorium.”

### 7. Theme customization

Ship three polished presets:

1. **Reading Room** — warm paper and evergreen; the default.
2. **After Hours** — forest charcoal and soft ivory.
3. **Clear Day** — neutral white and charcoal for users who prefer less
   atmosphere.

Keep reader presets separate: Paper, White, Sepia, Dark, and Custom.

Marketplace themes should customize the same design-token and component system
rather than replacing navigation conventions or breaking accessibility.

**The guiding rule: Lorehaven should look inviting when you arrive and
disappear when you start reading.**

---

## Implementation notes

The palette above is authoritative. `frontend/src/styles/tokens.css` is the
single source of truth for these values; component CSS must reference tokens
(`var(--color-accent)`) rather than literal hex codes, so that marketplace
themes and user presets can retarget the same component system
(spec §20, “Themes”).

Reader appearance is deliberately *separate* from the site theme: the reader
sets its own preset (Paper / White / Sepia / Dark / Custom) on a scoped
container, so a dark interface can still show a warm-paper reading page.
