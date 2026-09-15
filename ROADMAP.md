# 🏁 Pitwall Roadmap

Pitwall is evolving toward a simple goal:

> **Give developers a clear view of their AI-assisted workspace, so they can understand what their agents are doing, stay oriented, and remain in control.**

Pitwall starts with Omarchy, but the longer-term vision is to make it useful across Linux environments and AI coding workflows.

This roadmap describes the direction of the project rather than promising specific release dates. Priorities may change as we learn from people actually using Pitwall.

---

## 🟢 Now: Make Pitwall great on Omarchy

The first priority is making the current Omarchy experience reliable, useful and pleasant.

### Workspace awareness

- Reliable detection of active projects and sessions
- Clear visibility of AI coding agents
- Better understanding of relationships between agents and projects
- Useful activity and status information
- Reliable handling of completed and disappearing sessions

### AI workspace summaries

- Clear summaries of what's happening
- Better identification of important changes
- Useful "what happened while I was away?" context
- Keep AI context focused and bounded
- Make AI assistance optional and configurable

### Human control

- Clear, explicit actions
- Easy ways to focus, resume or stop work
- Avoid unexpected changes to the user's environment
- Make the boundary between observation and action obvious

### Notifications

- Surface events that actually matter
- Reduce unnecessary notifications
- Make it easy to understand why something needs attention

### Reliability and performance

- Fast startup
- Low background resource usage
- Reliable session detection
- Robust behaviour when terminals or agents disappear
- Simple installation and removal

---

## 🔵 Next: Make Pitwall useful across AI coding workflows

Pitwall should not require developers to change the AI coding tools they already use.

The next stage is broader compatibility with different agents and workflows.

Possible areas include:

- OpenCode
- Claude Code
- Codex
- other AI coding agents
- agent teams and multi-agent workflows
- long-running autonomous tasks
- human + AI handoff workflows

The goal is not to become another coding agent.

**Pitwall should remain the layer that helps you understand and manage the work around your agents.**

---

## 🟣 Next: Workspace memory and continuity

AI coding sessions can be long, fragmented and easy to lose track of.

Pitwall is exploring better ways to preserve useful workspace context without creating unnecessary data or complexity.

Potential capabilities include:

- workspace checkpoints
- session summaries
- project state
- handoff information
- "continue where I left off" workflows
- compact context for restarting AI sessions
- better awareness of what changed since the last session

The principle is:

> **Remember what matters, without remembering everything.**

---

## 🟠 Beyond Omarchy: More Linux environments

Omarchy is where Pitwall starts.

The longer-term goal is to make the core of Pitwall useful across Linux rather than tying it permanently to one desktop environment.

Potential targets include:

- Ubuntu
- Arch-based distributions
- Kali Linux
- GNOME
- KDE Plasma
- other Linux desktop environments

This work will favour a platform-independent core with environment-specific integrations where necessary.

For example:

    Pitwall
       |
       +-- Core workspace awareness
              |
              +-- Omarchy / Quickshell
              +-- GNOME
              +-- KDE
              +-- Other Linux environments

The exact architecture will evolve as support for additional environments is developed.

---

## 🟡 Open ecosystem

As Pitwall grows, we'd like it to be easy for others to extend it.

Possible areas include:

- agent integrations
- desktop integrations
- workspace integrations
- notification providers
- UI extensions
- workflow recipes
- community plugins
- developer tooling

The goal is to make Pitwall useful as an **open platform for AI-assisted development awareness**, rather than a closed application.

---

## 🔐 Privacy and local-first development

Privacy is part of the architecture, not an afterthought.

As new capabilities are added, Pitwall will continue to favour:

- local processing where practical
- no mandatory cloud service
- no mandatory account
- minimal data collection
- bounded AI context
- explicit user control
- transparent behaviour

New features should justify any additional data collection or external dependency.

---

## 🧪 Ideas we're exploring

Some ideas are intentionally still experimental.

These may include:

- AI context-window awareness
- recommendations when an agent's context becomes large
- automatic creation of compact project handoff documents
- easier restarting of AI agents with fresh context
- better coordination between multiple agents
- workspace-level AI conversations
- reusable human + AI workflows
- richer project and session history
- smarter attention and notification management

These are areas of exploration, not commitments to a particular implementation.

---

## 🤝 How you can help

The roadmap is not something the core team has to build alone.

Contributions are especially useful around:

- Omarchy integration
- Linux compatibility
- GNOME and KDE support
- AI agent integrations
- UI/UX
- accessibility
- performance
- reliability
- privacy
- documentation
- testing
- real-world workflows

If you are interested in an area that isn't listed here, open an issue or discussion and tell us what you have in mind.

---

## 🌱 The bigger picture

AI coding is moving from:

    One developer
          +
    One coding assistant

toward:

    One developer
          +
    Multiple AI agents
          +
    Multiple projects
          +
    Long-running work
          +
    Human decisions

As that happens, simply having more AI agents is not enough.

Developers need **awareness, continuity and control**.

That's the space Pitwall is exploring.

**Today: Omarchy.**

**Tomorrow: Linux.**

**Eventually: wherever AI-assisted development happens.**

---

## Status

Pitwall is actively evolving.

The roadmap represents the direction we're exploring, not fixed release commitments.

If you'd like to help shape that direction, contributions and ideas are welcome.

**Welcome to the Pitwall. 🏁**
