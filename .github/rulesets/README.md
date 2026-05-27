# Rulesets

JSON snapshots of the branch protection rulesets configured for this
repository. GitHub does not auto-apply these from the repo — they exist
as documentation and as a restore path via the REST API.

## Files

- `main.json` — protection for `refs/heads/main` (signed commits,
  required status checks, PR before merge, no force-push, no deletion,
  github-pages deployment must succeed)

## Restore (or apply elsewhere)

```bash
gh api \
  --method POST \
  -H "Accept: application/vnd.github+json" \
  /repos/{owner}/{repo}/rulesets \
  --input .github/rulesets/main.json
```

## Export current state

```bash
# List rulesets to find the id
gh api /repos/{owner}/{repo}/rulesets

# Export one
gh api /repos/{owner}/{repo}/rulesets/{id} > .github/rulesets/main.json
```

When you change rules in the UI, re-export and commit to keep this file
honest.

## Conventions

- `conditions.ref_name.include` uses `~DEFAULT_BRANCH` rather than a hard
  `refs/heads/main`. If `main` is ever renamed the ruleset still applies.
- The exported file carries `id`, `source`, `source_type` — these are
  read-only and ignored on POST restore. Leaving them in keeps the
  re-export → commit diff stable.
- All required status checks pin `integration_id: 15368` (the GitHub
  Actions app), so a third-party check with the same name can't satisfy
  the rule by accident.
