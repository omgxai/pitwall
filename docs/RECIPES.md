# Pitwall Recipes

> Reusable open-source development workflows for humans and AI agents.
> Small composable documents. Git-native. No marketplace, no accounts,
> no cloud, no paid anything.

## What a recipe is

A recipe describes **how to run a development workflow**, not a prompt
dump:

- roles (who does what: master, UI worker, QA worker, …)
- responsibilities and file ownership
- handoffs (how work moves between roles)
- project memory (logs, checkpoints, state files)
- coordination rules (shared worktree discipline, staging rules)
- approval boundaries (what needs a human click)
- Git workflow (branches, commits, PRs)
- testing workflow (what must pass before merge)
- Pitwall integration (status, checkpoints, Resume, summaries)

## What a recipe is not

- a giant prompt dump
- a proprietary marketplace listing
- a hosted service or login
- a way to sell access to agents

## Creating a recipe

1. Run the workflow for real (recipes document practice, not theory).
2. Write it small: one `README.md` plus role files if needed.
3. Name the ack/nak explicitly: what the human approves, what agents
   must never do alone.
4. Test it on a second project before sharing.

Suggested layout for larger recipes (optional, not required):

```
recipes/<name>/
    README.md
    master.md
    roles/
        ui.md
        backend.md
```

## Sharing a recipe

Fork Pitwall, add your recipe under discussion in an issue or PR,
iterate in the open. See `CONTRIBUTING.md`. Good recipes get linked
from the roadmap; great ones earn examples in this file.

## Example directions (not implementations)

AI Coding Team · Solo AI Developer · Open Source Maintainer ·
Full Stack Team · Research Team · QA/Test Team · Startup Project.

These are starting points for community experimentation — pick one,
run it, write down what actually worked.

## Relation to Pitwall

Recipes are methodology; Pitwall is instrumentation. A recipe tells the
team how to work; Pitwall shows the team what is happening. Either is
useful alone; together they close the loop.
