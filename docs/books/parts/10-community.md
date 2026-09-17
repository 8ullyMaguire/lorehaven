# Part 10 — Comments, forums, groups, messaging and events

Checkpoint: `v0.12-discovery`

This is the part where the site gets a social surface, and therefore the part
where the abuse surface grows fastest. Build it with the moderation tooling in
the same part — not later. "We will add reporting when it becomes a problem" is
how a problem becomes unmanageable.

## 1. Checkpoint

```bash
git checkout v0.12-discovery
```

## 2. What will work by the end

```bash
# Paragraph-anchored comment on a chapter
curl -X POST localhost:8080/api/v1/works/$WORK/comments \
  -d '{"body":"this line is perfect","anchor":{"kind":"paragraph","chapter":"…","value":12}}'

# A forum topic with posts, a group with membership, and a notification.
curl -X POST localhost:8080/api/v1/forums/$F/topics -d '{"title":"…","body":"…"}'
curl -X POST localhost:8080/api/v1/groups -d '{"name":"…"}'
curl localhost:8080/api/v1/notifications
```

## 3. Concepts

- **An anchor is a pointer into a version of a thing, and it can rot.** Design
  for the case where the paragraph no longer exists.
- **Forums are a different shape from comments**: flat topics, ordered posts,
  per-topic subscriptions.
- **Groups are permission scopes**, not just lists of people.
- **Messaging is private and must stay out of every other system**: no counts in
  someone else's view, no payloads in webhooks, no content in admin lists.
- **Notifications are derived from the outbox**, so a notification can never
  exist for an event that did not commit.
- **Events and challenges are time-bounded**: they open, they accept entries, they
  close, and after they close nothing changes them.

## 4. Commands

```bash
lorehaven migrate        # applies 0013_community, 0014_events, 0023_notifications, 0027_comment_anchors
cargo test -p lorehaven-app --test milestone_12
cargo test -p lorehaven-app --test milestone_13
```

## 5. Exact file changes

| Path | What it is |
|---|---|
| `migrations/sqlite/0013_community.sql` | comments, forums, topics, posts, groups, memberships, messages, presence |
| `migrations/sqlite/0014_events.sql` | collections, challenges, requests, wishlists, writing events |
| `migrations/sqlite/0023_notifications.sql` | notifications and their read state |
| `migrations/sqlite/0027_comment_anchors.sql` | `anchor_kind`, `anchor_value`, `anchor_chapter_id` |
| `crates/domain/src/anchor.rs` | anchor validation rules |
| `crates/domain/src/community.rs` | thread rules, ordering, group permissions |
| `crates/domain/src/events.rs` | event lifecycle |
| `crates/db/src/community.rs`, `db/collaboration.rs`, `db/notifications.rs`, `db/events.rs` | storage |
| `crates/app/src/routes/community.rs`, `routes/events.rs`, `routes/notifications.rs` | the doors |
| `frontend/src/routes/Community.svelte`, `ForumCategory.svelte`, `ForumTopic.svelte`, `Notifications.svelte` | the pages |

## 6. The code that matters

### Anchors, and their two failure modes

```rust
// crates/domain/src/anchor.rs
// paragraph anchor: a chapter id and a non-negative integer
// timestamp anchor: HH:MM:SS(.fff), no chapter
```

The rules exist to stop the two failures anchors always have:

1. **A paragraph anchor without a chapter** is meaningless — paragraph twelve of
   what? Make it invalid rather than defaulting to chapter one.
2. **A timestamp anchor with a chapter id** is a contradiction; a timestamp is
   inside a time-based medium, a chapter is inside a text one.

And then the honest part: an anchor can point at a paragraph that no longer
exists, because the author edited the chapter. Decide the behaviour and write it
in the UI:

```text
anchor resolves   → show the comment inline with the paragraph
anchor does not   → show the comment with "on a paragraph that is no longer here"
```

Never silently drop the comment, and never silently re-attach it to whatever is
now at that offset.

### The comment door, end to end

Every comment post does five things in one transaction:

```text
1. validate the anchor (domain rule)
2. insert the comment
3. classify it (Part 6) and store the outcome
4. write the outbox event (Part 5)
5. side effects — notifications, counters — derive from the outbox, not here
```

If step 3 or 4 is outside the transaction, you get a visible hostile comment, or
a notification for a comment that does not exist. Both are bugs you will only see
under load.

### Forum topics versus comments

They look similar and should not share code:

| | comments | forum posts |
|---|---|---|
| ordering | by anchor, then time | strictly by time |
| editing | allowed, with history | allowed, marked as edited |
| nesting | none | none (flat, deliberately) |
| subscriptions | per work | per topic |
| moderation | per work, by the author | per board, by moderators |

Flat forums are a decision worth making explicitly: threading produces the
worst-behaved comment sections on the internet, and you are already building a
filter for a reason.

### Messaging privacy rules, as code

- A message row has a sender and a recipient pseud. There is no "read by admin".
- An administrator tool that lists messages must be audited and must show
  metadata only (who, when, how many) — never bodies.
- Blocking applies to messages **silently**: the sender is not told.
- Deleting a message deletes it for both parties; there is no "delete for me"
  state that the other party can still quote from the database.

### Notifications from the outbox

```text
outbox event "comment.delivered" → notification rows for subscribers
```

Because the event is written in the same transaction as the comment, a
notification cannot exist for a comment that rolled back. Delivery to email or
push is a job (Part 5) with retries, and each channel's failures are visible to
the person who configured it.

### Events and challenges

```text
state: draft → open → closed → archived
entries are appended while open; after closed, no writes are accepted
```

The rule that matters: after a challenge closes, the entry list is frozen.
Allowing late edits makes every result disputable.

## 7. Tests

`milestone_12.rs` (community):

- a paragraph anchor without a chapter, and a timestamp anchor with one, are both
  400s;
- an anchored comment appears in the chapter's comment list in offset order;
- a comment whose anchor no longer resolves still appears, flagged, and is not
  silently moved;
- a blocked pseud cannot comment on the blocker's work, and the refusal does not
  reveal the block;
- a group's private content is 404 to a non-member;
- a message body never appears in any notification, webhook or admin payload.

`milestone_13.rs` (events):

- an entry submitted after a challenge closes is refused;
- closing is idempotent;
- a collection's ordering is stable across reads.

## 8. Expected UI behaviour

- Commenting on a paragraph highlights the paragraph while you type.
- Forum topics show their last activity honestly, and nothing is "pinned" without
  a moderator action.
- Notifications clear individually and in bulk, and a cleared one stays cleared.
- A direct message thread loads in order and marks read once.

## 9. Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| Anchor drifts after editing a chapter | anchoring by page position, not a paragraph index | anchor on the chapter's paragraph model |
| Comment appears twice | client retry without an idempotency key | accept a client token, unique per (comment, token) |
| Notification for a deleted comment | notifications built after commit from a query | derive from the outbox event |
| Forum topic list is slow | unindexed `ORDER BY last_post_at` | index on `(board_id, last_post_at)` |
| A block leaks through messaging | the send door checks the sender's blocks only | check both directions |

## 10. Consequences

- **You now hold private conversations.** Encryption at rest, retention limits,
  and an explicit policy for law-enforcement requests are the cost of this part.
  If you are not prepared to write that policy, do not ship messaging.
- **Presence is location data.** "Online now" tells anyone when a person is at
  their keyboard. Default it off, and never show it on a profile the user did not
  enable.
- **Every social surface multiplies the filter's importance.** Part 6 protects a
  writer from a comment section; a group chat needs the same protection, plus
  moderators who are accountable (Part 11).

## 11. Checkpoint

```bash
git tag v0.13-community
```

Verified by `milestone_12.rs` and `milestone_13.rs` plus a browser pass with two
accounts: comment with an anchor, edit the chapter, reload, and confirm the
comment is flagged rather than moved.
