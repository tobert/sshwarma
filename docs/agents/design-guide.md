# Agent Design Document Guide

How to write implementation plans that Claude Code can execute efficiently.

---

## Philosophy

**Focus over length.** The problem isn't token count — it's domain bleeding. When a primitives doc mentions rendering, Claude starts thinking about rendering, which expands context. Keep each document in its lane.

**Signatures, not implementations.** Type definitions and method signatures tell Claude *what* to build. Full implementations tell Claude what you *already* built. Let the agent write the code.

**Clear prompts for handoff.** Each task file should work as a standalone prompt. "Read 01-primitives.md and proceed" should be enough.

---

## Document Structure

```
docs/agents/plans/{feature}/
├── README.md      # Activation, tracking, architecture
├── DETAIL.md      # Design rationale (read during revisions)
├── 01-first.md    # Task 1
├── 02-second.md   # Task 2
└── ...
```

| Document | Purpose | When to Read |
|----------|---------|--------------|
| README.md | Orient, track progress, see architecture | Every session |
| DETAIL.md | Understand *why* decisions were made | Revision sessions |
| NN-task.md | Execute one implementation task | Implementing that module |

---

## README.md Template

```markdown
# Feature Name

**Location:** `src/module.rs` or `src/module/`
**Status:** Design Complete | In Progress | Complete

---

## Progress Tracking

| Task | Status | Parallel Group | Notes |
|------|--------|----------------|-------|
| 01-primitives | pending | A | No dependencies |
| 02-types | pending | A | No dependencies |
| 03-core | pending | B | Depends on 01, 02 |

## Success Metrics

- [ ] All tests pass
- [ ] Feature works end-to-end
- [ ] No new warnings

## Execution Flow

\`\`\`mermaid
graph TD
    subgraph A[Group A - parallel]
        A1[01-primitives]
        A2[02-types]
    end
    subgraph B[Group B]
        B1[03-core]
    end
    A1 --> B1
    A2 --> B1
\`\`\`

## Agent Dispatch

### Group A (2 agents, parallel)
\`\`\`
Agent 1: "Read 01-primitives.md and implement."
Agent 2: "Read 02-types.md and implement."
\`\`\`

### Output Format
When complete, report:
- Files modified (paths)
- Tests added/passing
- Blockers or follow-up discovered
- Key context the orchestrator should know

## Documents

| Document | Focus | Read When |
|----------|-------|-----------|
| [01-primitives.md](./01-primitives.md) | Core types | Implementing primitives |
| [02-types.md](./02-types.md) | Type definitions | Implementing types |

## Open Questions

- [ ] Unresolved design question?
```

---

## DETAIL.md Template

```markdown
# Feature Design Rationale

**Purpose:** Deep context for revision sessions. Read when you need to understand *why*.

---

## Why [Key Decision 1]?

[Explanation of the decision, alternatives considered, trade-offs]

## Why [Key Decision 2]?

[...]

## Cross-Cutting Concerns

### [Concern 1]

[How this affects multiple modules]

### [Concern 2]

[...]

## Open Questions

| Question | Context | Status |
|----------|---------|--------|
| - | - | - |

## Rejected Alternatives

| Alternative | Why Rejected |
|-------------|--------------|
| X | Caused Y problem |
```

**Target: Unlimited — only read during deep revision sessions**

---

## Task File Template

```markdown
# NN: Task Name

**File:** `src/module.rs`
**Focus:** [One domain only]
**Dependencies:** 01-other-task, 02-another-task
**Unblocks:** 04-dependent-task

---

## Task

[Clear instruction: what to create, what file(s) to write]

**Why this first?** [Explain ordering rationale — what this enables, what depends on it]

**Deliverables:**
1. [Specific file with specific contents]
2. [Specific functionality working]
3. [Specific tests passing]

**Definition of Done:**
```bash
cargo fmt --check
cargo clippy -- -D warnings
cargo check
cargo test
```

## Out of Scope

- ❌ [Adjacent work] — that's task NN
- ❌ [Other adjacent work] — that's task MM

Focus ONLY on [this domain].

---

## [Relevant External Crate] Patterns

```rust
// Key API usage the agent needs to know
use external_crate::Thing;
let x = Thing::new();
```

---

## Types

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MyType {
    pub field: Type,
}

pub enum MyEnum {
    Variant1,
    Variant2 { data: String },
}

pub trait MyTrait: Send + Sync {
    fn method(&self, arg: Type) -> Result<Output>;
}
```

---

## Methods to Implement

**Construction:**
- `new(...) -> Self`
- `from_x(...) -> Self`

**Core:**
- `do_thing(&self, ...) -> Result<...>`
- `other_thing(&mut self, ...)`

**Queries:**
- `get_x(&self) -> &X`
- `find_by_y(&self, y: Y) -> Option<&Z>`

---

## Acceptance Criteria

- [ ] Types compile with derives as shown
- [ ] [Specific behavior] works correctly
- [ ] Tests cover [specific scenarios]
- [ ] [Edge case] handled
```

**Target: 100-200 lines**

---

## What Goes Where

| Content | Location | Example |
|---------|----------|---------|
| Type definitions | Task file | `pub struct Foo { ... }` |
| Trait definitions | Task file | `pub trait Bar { ... }` |
| Method signatures | Task file | `fn baz(&self) -> Result<X>` |
| Method bodies | Nowhere | Agent writes these |
| External crate patterns | Task file | `toposort(&graph, None)` |
| Design rationale | DETAIL.md | "We chose X because Y" |
| Rejected alternatives | DETAIL.md | "We didn't use Z because..." |
| Cross-cutting concerns | DETAIL.md | "Error handling affects all modules" |
| Progress tracking | README.md | Status table with parallel groups |
| Execution flow | README.md | Mermaid DAG |

---

## Focus Rules

### One Domain Per File

Each task file owns one domain. If you find yourself explaining another domain to make sense of this one, stop — you're bleeding.

**Good:** "Takes a `Player` as input"
**Bad:** "Parses the SSH channel using russh's ChannelStream with AsyncRead..."

### Scope Boundaries

Every task file needs a "Do not" section:

```markdown
**Do not** implement HUD rendering — that's task 04.
**Do not** implement actual LLM calls — use the existing llm module.
```

### Reference, Don't Explain

When task 04 depends on task 01's types:

```markdown
**Dependencies:** 01-primitives
```

Don't re-explain the types. The agent can read the dependency's task file if needed.

---

## sshwarma-Specific Patterns

### Async/Blocking Pattern

When calling `blocking_read()` or `blocking_write()` on `RwLock` from async contexts, wrap with `block_in_place()`:

```rust
// BAD: panics in async context
let world = self.state.world.blocking_write();

// GOOD: safe in async context
let world = tokio::task::block_in_place(|| self.state.world.blocking_write());
```

Include this pattern in task files that touch `SharedState`.

### Error Handling

Use `anyhow::Result` for all fallible operations:

```rust
use anyhow::{Context, Result};

fn load_config(path: &Path) -> Result<Config> {
    let content = std::fs::read_to_string(path)
        .context("Failed to read config file")?;
    toml::from_str(&content)
        .context("Failed to parse config")
}
```

### Key Crates

| Crate | Usage | Pattern Example |
|-------|-------|-----------------|
| russh | SSH server | `session.data(channel, data.into()).await` |
| rmcp | MCP client/server | `client.call_tool(name, args).await` |
| rig | LLM orchestration | `agent.chat(prompt, chat_history).await` |
| mlua | Lua scripting | `lua.scope(\|scope\| { ... })` |
| rusqlite | SQLite | `conn.execute(sql, params![...])` |

---

## External Context

Use Exa or web search to find key patterns for dependencies, then embed them in task files:

```rust
// russh: sending data to client
session.data(channel_id, CryptoVec::from(bytes)).await?;

// rig: building an agent with tools
let agent = openai_client
    .agent("gpt-4")
    .tool(MyTool::new())
    .build();

// rmcp: calling an MCP tool
let result = client
    .call_tool(CallToolRequestParam {
        name: "tool_name".into(),
        arguments: Some(args),
    })
    .await?;
```

This saves agents from searching and keeps them focused.

---

## Definition of Done

Every task file must include:

```markdown
**Definition of Done:**
```bash
cargo fmt --check
cargo clippy -- -D warnings
cargo check
cargo test
```
```

For feature-gated code, add both paths:

```bash
cargo check
cargo check --features feature_name
```

---

## Acceptance Criteria

Use checkboxes for functional requirements:

```markdown
## Acceptance Criteria

- [ ] `new()` creates valid instance
- [ ] Serialization round-trips correctly
- [ ] Error case returns `Err`, not panic
- [ ] Edge case X handled
```

These are *what* must work. Definition of Done is *how* to verify the code is ready.

---

## Parallel Execution

Plans should enable a coordinating agent to dispatch parallel workers and synthesize results efficiently.

### Dependency Graph

Use Mermaid in README.md to visualize the execution DAG:

```markdown
## Execution Flow

\`\`\`mermaid
graph TD
    subgraph A[Group A]
        A1[01-primitives]
        A2[02-types]
        A3[03-helpers]
    end
    subgraph B[Group B]
        B1[04-core]
        B2[05-io]
    end
    subgraph C[Group C]
        C1[06-integration]
    end
    A1 --> B1
    A2 --> B1
    A2 --> B2
    A3 --> B2
    B1 --> C1
    B2 --> C1
\`\`\`
```

Mermaid is parseable by future agents, renders on GitHub, and makes dependencies explicit.

### Task Dependencies

Each task file declares what it needs and what it unblocks:

```markdown
**Dependencies:** 01-primitives, 02-types
**Unblocks:** 05-io, 06-integration
```

### Agent Dispatch Section

Include ready-to-use prompts in README.md:

```markdown
## Agent Dispatch

### Group A (3 agents, parallel)
\`\`\`
Agent 1: "Read 01-primitives.md and implement."
Agent 2: "Read 02-types.md and implement."
Agent 3: "Read 03-helpers.md and implement."
\`\`\`
```

### Output Discipline

Instruct workers on efficient reporting. The orchestrator synthesizes multiple outputs — structured reports reduce overhead.

Add to agent prompts:
```
When complete, report:
- Files modified (paths)
- Tests added/passing
- Blockers or follow-up discovered
- Key context the orchestrator should know
```

### Rich Context for Workers

Give workers everything they need to work confidently:
- Full type signatures and examples
- Links to related files
- Design rationale where helpful

### Ask Early, Ask Often

**Asking clarifying questions saves everyone time.** Don't guess — ask.

Subagents can ask you questions directly via AskUserQuestion. Your answers go to their context, not the orchestrator's. This means:
- You can make decisions at implementation time
- Clarifications stay with the worker who needs them
- The orchestrator stays lean — just dispatching and synthesizing

Good reasons to ask:
- Multiple valid approaches — which do you prefer?
- Ambiguous requirement — what's the actual intent?
- Discovered complexity — should we simplify scope?
- Found related issues — fix now or note for later?

A 30-second question beats a 30-minute wrong implementation.

---

## Checklist Before Finalizing Plan

**README.md:**
- [ ] Progress tracking table with parallel groups
- [ ] Mermaid execution DAG
- [ ] Agent dispatch prompts
- [ ] Success metrics (checkboxes)
- [ ] Open questions (checkboxes)

**DETAIL.md:**
- [ ] Explains *why* for key decisions
- [ ] Documents rejected alternatives

**Task Files:**
- [ ] Dependencies and Unblocks declared
- [ ] Task section with clear prompt
- [ ] Definition of Done (fmt/clippy/check/test)
- [ ] Acceptance Criteria checkboxes
- [ ] Rich context (types, examples, rationale)

**Overall:**
- [ ] No full implementations anywhere
- [ ] No domain bleeding between task files
