# Lorehaven Configuration Reference

Every instance is configured through `lorehaven.toml` (spec §38). This file
documents each section and its keys, with defaults, type, and purpose.

Sections are applied in the order shown here; `deny_unknown_fields` means a
typo in any key is a startup error rather than a silent misconfiguration.

## `[site]`

The identity of the instance.

| Key | Type | Default | Purpose |
|-----|------|---------|---------|
| `name` | `string` | `"Lorehaven"` | Display name across the UI |
| `base_url` | `string` | auto | Canonical URL (auto = bind + port) |
| `contact_email` | `string?` | `None` | Admin contact shown on error pages |
| `topics` | `[string]` | `[]` | Public discovery topics |

## `[server]`

How the process listens.

| Key | Type | Default | Purpose |
|-----|------|---------|---------|
| `bind` | `string` | `"127.0.0.1"` | Listen address |
| `port` | `int` | `8080` | Listen port |
| `max_body_bytes` | `int` | `2097152` | Max request body (2 MiB) |
| `request_timeout_secs` | `int` | `30` | Per-request timeout |

## `[database]`

Connection string and pool settings.

| Key | Type | Default | Purpose |
|-----|------|---------|---------|
| `url` | `string` | `sqlite://./data/lorehaven.sqlite` | DB connection string |
| `max_connections` | `int` | `5` / `10` | Pool size (dev / prod) |
| `acquire_timeout_secs` | `int` | `10` | Pool acquire timeout |

## `[storage]`

Filesystem layout for blobs.

| Key | Type | Default | Purpose |
|-----|------|---------|---------|
| `root` | `string` | `"./data"` / `"/var/lib/lorehaven"` | Blob root (dev / prod) |

## `[security]`

Cookie, session, and CSRF settings.

| Key | Type | Default | Purpose |
|-----|------|---------|---------|
| `cookie_secure` | `bool` | `false` / `true` | Secure cookie flag (dev / prod) |
| `session_ttl_days` | `int` | `30` | Session lifetime |
| `csrf_required` | `bool` | `true` | CSRF protection |
| `trust_proxy` | `bool` | `false` | Trust `X-Forwarded-*` headers |

## `[logging]`

What is logged and how.

| Key | Type | Default | Purpose |
|-----|------|---------|---------|
| `filter` | `string` | `"info,lorehaven_app=debug"` | Log filter |
| `format` | `string` | `"pretty"` / `"json"` | Log format (dev / prod) |

## `[assets]`

Where static assets live.

| Key | Type | Default | Purpose |
|-----|------|---------|---------|
| `root` | `string?` | unset | Custom asset directory |

## `[administration]`

Operator and webhook settings.

| Key | Type | Default | Purpose |
|-----|------|---------|---------|
| `operator_account_id` | `int?` | `None` | Account ID allowed on `/admin` routes |
| `webhook_timeout_secs` | `int` | `10` | Webhook request timeout |
| `webhook_max_attempts` | `int` | `5` | Webhook delivery attempts |
| `webhook_base_delay_ms` | `int` | `500` | Webhook backoff base |
| `webhook_allowed_hosts` | `[string]` | `[]` | Webhook host allowlist |

## `[dev]`

Development-only toggles.

| Key | Type | Default | Purpose |
|-----|------|---------|---------|
| `seed_enabled` | `bool` | `true` | Allow `lorehaven seed` to write fixtures (prod = false) |

## `[imports]`

Source fetching and CAPTCHA settings.

| Key | Type | Default | Purpose |
|-----|------|---------|---------|
| `honour_robots` | `bool` | `true` | Respect robots.txt |
| `solver_url` | `string?` | `None` | CAPTCHA solver URL |
| `archive_fallback` | `bool` | `false` | Fallback to archive.org |

## `[exports]`

Export file retention and CTA settings (spec §38, §42).

| Key | Type | Default | Purpose |
|-----|------|---------|---------|
| `retention_days` | `int` | `7` | Days to keep export output (`0` = forever) |
| `grant_ttl_secs` | `int` | `3600` | Download grant lifetime |
| `cta_placement` | `string` | `"per_chapter"` | CTA placement: `per_chapter`, `per_work`, or `off` |
| `cta_html` | `string?` | unset | Custom CTA HTML (sanitized) |
| `cta_quorum` | `int` | `2` | Curator quorum for work-level CTA exemption |

## `[bulk_export]`

Bulk export limits (spec §38).

| Key | Type | Default | Purpose |
|-----|------|---------|---------|
| `max_items` | `int` | `50` | Maximum works in a bulk bundle |
| `max_bytes` | `int` | `1073741824` | Maximum bytes in a bulk bundle (1 GiB) |

## `[revisions]`

Source revision cache settings (spec §38).

| Key | Type | Default | Purpose |
|-----|------|---------|---------|
| `ttl_secs` | `int` | `604800` | Revision cache TTL (7 days) |

## `[jobs]`

Job queue settings (spec §38).

| Key | Type | Default | Purpose |
|-----|------|---------|---------|
| `terminal_retention_days` | `int` | `30` | Days to keep terminal jobs |

## `[library]`

Library update-check settings (spec §38).

| Key | Type | Default | Purpose |
|-----|------|---------|---------|
| `update_check_retention_days` | `int` | `90` | Days to keep update-check records |
| `check_batch` | `int` | `50` | Items per library update job |

## `[tts]`

Text-to-speech engine settings.

| Key | Type | Default | Purpose |
|-----|------|---------|---------|
| `engine` | `string` | `"piper"` | Engine name (`piper`, `silent`) |
| `piper_path` | `string?` | `None` | Piper binary path |
| `piper_voice_model` | `string?` | `None` | Piper voice model |
| `default_voice` | `string?` | `None` | Default voice |
| `monthly_spend_cap_cents` | `int?` | `None` | Monthly spend cap |

## `[forum]`

Forum, vote budget, and karma settings (spec §35).

| Key | Type | Default | Purpose |
|-----|------|---------|---------|
| `work_discussion_default` | `string` | `"thread_only"` | Default discussion mode |
| `vote_budget` | `map<string,int>` | (see spec §35.2) | Vote budget rungs by trust level |
| `karma_decay_percent` | `int` | `5` | Monthly karma decay % |
| `meta_mod_points` | `int` | `1` | Points per meta-mod verdict |
| `meta_mod_min_verdicts` | `int` | `3` | Min verdicts to elect moderator |
| `min_vote_weight_bp` | `int` | `100` | Min vote weight (basis points) |

## `[browse]`

Browsing defaults (spec §43).

| Key | Type | Default | Purpose |
|-----|------|---------|---------|
| `default_sort` | `string` | `"for-you"` | Default sort for signed-in readers |
| `anonymous_sort` | `string` | `"top"` | Default sort for anonymous visitors |
| `surface_defaults` | `map<string,string>` | unset | Per-surface overrides |

## `[weighting]`

Ranking weights (spec §16.16).

| Key | Type | Default | Purpose |
|-----|------|---------|---------|
| `mode` | `string` | `"trust_taste_contribution"` | Weighting mode |
| `taste_floor` | `float` | `0.75` | Taste multiplier floor |
| `taste_ceiling` | `float` | `1.25` | Taste multiplier ceiling |
| `contribution_floor` | `float` | `1.0` | Contribution multiplier floor |
| `contribution_ceiling` | `float` | `2.0` | Contribution multiplier ceiling |
| `contribution_window_days` | `int` | `180` | How far back contribution counts |
| `demand_diversity_percent` | `int` | `20` | Fraction of surfaced demand with no boost |

## `[discovery]`

Discovery settings.

| Key | Type | Default | Purpose |
|-----|------|---------|---------|
| `taste_sources` | `[table]` | `[{ kind = "admin" }]` | Taste sources and members |
| `taste_source_min_members` | `int` | `5` | Cohort-size floor |

## `[directory]`

Resource directory settings (spec §39).

| Key | Type | Default | Purpose |
|-----|------|---------|---------|
| `page_size` | `int` | `50` | Entries per page |
| `require_approval` | `bool` | `true` | Submissions need approval |
| `extra_categories` | `[string]` | `[]` | Operator-added categories |
| `weighting` | `string` | `"trust_and_taste"` | Vote weighting mode |
| `trust_vote_weights` | `[float]` | `"0.5,0.75,1.0,1.25,1.5,1.75,2.0"` | TL0–TL6 multipliers |
| `taste_floor` | `float` | `0.75` | Taste floor |
| `taste_ceiling` | `float` | `1.25` | Taste ceiling |

## `[rate_limits]`

Rate-limit quotas (spec §3.8). Each bucket has `burst` and `per_minute`.

| Key | Burst | Per minute |
|-----|-------|-----------|
| `auth` | 5 | 20 |
| `write` | 10 | 60 |
| `search` | 20 | 120 |
| `export` | 3 | 10 |
| `default` | 30 | 180 |

## `[accounts]`

Account settings.

| Key | Type | Default | Purpose |
|-----|------|---------|---------|
| `registration_open` | `bool` | `true` | Allow new registrations |

## `[age]`

Age-gate settings.

| Key | Type | Default | Purpose |
|-----|------|---------|---------|
| `threshold` | `int` | `14` | Age of consent threshold |
| `guardian_workflow_enabled` | `bool` | `false` | Guardian authorization flow |

---

## Startup errors

A malformed value refuses to start with a clear error message. Examples:

- `retention_days = -1` → "exports.retention_days must be >= 0, got -1"
- `grant_ttl_secs = 0` → "exports.grant_ttl_secs must be > 0, got 0"
- Unknown TOML key → "unknown field `[section].foo`"

## Empty configuration

A configuration with all defaults omitted produces a working instance. The
shipped `lorehaven.toml.example` documents every setting with comments.
