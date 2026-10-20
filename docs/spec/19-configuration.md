# 19. The configuration contract

**Status:** reserved
**Milestone:** M1
**Source:** the corresponding section of the Darkhold specification

The self-hosting contract, inherited from lorehaven §38.

1. **Every limit and toggle is a configuration value** with a documented default, rather than a
   number in the code.
2. **Precedence: command line → environment (`DARKHOLD_*`) → file → documented default.**
3. **Validation at startup, not at use.** A misconfigured instance refuses to start and names the
   setting.
4. **`deny_unknown_fields`** on most sections — a typo is a startup error.
5. **No runtime reload.** A configuration change is a restart. No per-user configuration of
   instance-level behaviour. No feature flags for things that are always on or always off.
6. **`darkhold doctor`** reports the state of the machine: configuration, database, schema version,
   storage, converters, adapters and their reachability, the taste service's RSS headroom, and the
   current tier of each tier gate.
7. **Production is strict.** `serve` refuses to start if cookies would not be `Secure`, if CSRF
   protection is off, if the development seeder is enabled, if the public URL is not HTTPS, or if
   storage is still a development path. **Each refusal names the setting to change.**

**The configuration sections:** `server` · `database` · `storage` · `security` · `search` ·
`discovery` · `moods` · `tiers` · `theme` · `channels` · `ingestion` · `retention` · `community` ·
`guest` · `merge` · `consent` · `trust` · `governance` · `supporters` · `extensions` · `federation`
· `exchange` · `media` · `accessibility` · `jobs` · `telemetry` · `assets` · `limits`.

---
