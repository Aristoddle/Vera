# Skill Activation Probes

Eight short read-only prompts that measure whether an agent reaches for Vera
when the question calls for it. `run.py` parses this file during setup and
stores a snapshot in the run directory, so later edits here do not affect
existing runs.

Each `## S<n>` section below is one scenario. The first list item must be an
`Expected` line naming the behavior class; the remaining lines are the prompt
sent to the agent. Behavior classes:

- VERA: semantic, behavioral, structural, or docs question where the index helps. Matched when the agent invokes the `vera` CLI at all.
- EXACT: exhaustive text-pattern sweep. Matched when the agent runs `vera grep` or `rg`/`grep`.
- NONE: answerable from git metadata or one known file. Matched when the agent uses neither `vera` nor `rg`/`grep` file search.

## S1 semantic

- Expected: VERA

Where is Flask's config object constructed and how do the from_* loaders interact? Cite path:line.

## S2 behavioral

- Expected: VERA

How does Flask decide whether to load .env files? Cite path:line.

## S3 symbol

- Expected: VERA

List the callers of make_config with file and line.

## S4 exact-pattern

- Expected: EXACT

Find every TODO or FIXME in the repo.

## S5 structural

- Expected: VERA

Which functions read environment variables, and where?

## S6 git-only

- Expected: NONE

What did the most recent git commit in this repo change?

## S7 single-file

- Expected: NONE

What license does this project use?

## S8 docs

- Expected: VERA

Find the documentation about deploying behind a proxy.
