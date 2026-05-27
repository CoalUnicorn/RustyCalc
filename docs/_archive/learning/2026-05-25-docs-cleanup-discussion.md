# Docs Cleanup Discussion (May 25, 2026)

## Context

The `docs/` directory had grown to 32 files with no clear organizing
principle. Sat down with Claude to figure out what to archive, what to
keep, and what principle to follow going forward.

## Claude's take (as human reviewer)

Core principle: **Evergreen at root, dated material in subdirs.**

Specific decisions:
- Delete `guides/deepseek-api-headless.md` — not RustyCalc
- Archive `superpowers/` — untouched since May 6, no longer planned
- Keep `learning/` — journal entries are primary sources, different from the book. But rename consideration: `learning/` → `journal/` (not urgent)
- Keep `designs/` and `plans/` separate — they answer different questions (what vs how), even when they overlap on the same feature
- Don't reorganize root-level files — 8 files is fine, premature organization = premature abstraction
- Add cross-references between designs and their implementation plans

## What resonated

- "Reorgs break every external link/bookmark/grep that pointed at old paths"
- "Optimize for finding things, not for feeling tidy" — applies to code too
- The book/journal distinction: book is curated narrative for readers, journal is raw thinking for myself. Neither replaces the other
- "Delete the obviously-dead things first" — the 80/20 of cleanup

## What I did

- Deleted `docs/guides/deepseek-api-headless.md`
- Archived `docs/superpowers/` → `docs/_archive/superpowers/` with README
- Added Chapter 19.10 to the book capturing the philosophy
