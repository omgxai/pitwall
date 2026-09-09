# Pitwall AI Workforce Design

> Open-source direction. Humans stay responsible; agents stay independent
> tools; Pitwall provides shared awareness. No accounts, no cloud, no
> paid tiers — the pattern works fully locally.

## The pattern

```
Human (owns decisions, approvals, Coffee)
  ↓ coordinates
Project Master (human or coordinator agent: plan, delegate, integrate)
  ↓ specializes
Child workers (scoped roles: UI, backend, QA, docs…)
  ↓ produce
Git / project artifacts (the only shared truth)
  ↓ observed by
Pitwall (sessions, agents, checkpoints, Resume, summaries)
  ↓ feeds back to
Human visibility and control
```

## Rules that make it work

1. **Git is the source of truth.** Agent reports never override it.
2. **Roles are scoped.** Each worker owns named files/areas; the master
   integrates. No two workers edit the same critical file uncoordinated.
3. **Memory is local files.** A master log plus per-worker stories,
   date-first, committed with the work. Never a chat transcript as truth.
4. **Approval boundaries are explicit.** Destructive or costly actions
   (deploy, spend, delete, push to shared branches) need a human click.
5. **Resumption is cheap.** `.pitwall/STATE.md`-style checkpoints let any
   worker (human or AI) re-enter mid-stream without a handover meeting.
6. **Observation beats interrogation.** Pitwall shows who is working on
   what, so the master coordinates from evidence instead of asking.

## Verified example: the Pitwall Race Engineer run

During Pitwall's own M5 development, a coordinator session ran exactly
this pattern against this repository: a master log (`agent_master.md`),
scoped child roles (UI vs systems), file-based inbox/status coordination,
explicit staging discipline on a shared worktree, and `pitwall status`
dogfooding to see its own session. What is documented here is what was
actually practiced — single-agent operation included. The pattern scales
down to one human plus one agent, which is where most developers start.

## What Pitwall provides vs what agents provide

- Pitwall: session/agent discovery, checkpoints, Resume, summaries,
  human approval surfaces. Read-mostly, local-first.
- Agents (OpenCode, Claude Code, …): inference, provider auth, models,
  execution — their own vendors, keys, and billing, untouched by Pitwall.

## Contributing patterns

Got a team shape that works (solo + agent, maintainer + reviewers,
research spikes, QA lanes)? Write it up as a recipe — see
`docs/RECIPES.md` and `CONTRIBUTING.md`.
