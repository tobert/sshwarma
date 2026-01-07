# Journal — Quick Reference

Journals are per-room logs for notes, decisions, milestones, ideas, and questions.

## Entry Kinds

| Kind | Purpose | Example |
|------|---------|---------|
| `note` | General observations | "API returns 404 for missing users" |
| `decision` | Architectural choices | "Using PostgreSQL for persistence" |
| `milestone` | Progress markers | "Auth flow complete" |
| `idea` | Future exploration | "Could add WebSocket support" |
| `question` | Open questions | "How should we handle rate limits?" |

## SSH Commands

### Writing Entries
```
/note <text>        Add a note
/decide <text>      Record a decision
/idea <text>        Capture an idea
/milestone <text>   Mark a milestone
```

### Reading Entries
```
/journal            View recent entries (all kinds)
/journal decision   Filter by kind
/journal 10         Limit to last 10
```

## MCP Tools

### Writing
```json
journal_write({
  "room": "workshop",
  "kind": "decision",
  "content": "Using PostgreSQL for persistence",
  "author": "claude"
})
```

### Reading
```json
journal_read({
  "room": "workshop",
  "kind": "decision",
  "limit": 20
})
```

## Inspirations

Inspirations are a special journal feature — mood board items that inform the room's creative direction.

### SSH
```
/inspire            View current inspirations
/inspire <text>     Add new inspiration
```

When forking a room, journal entries become inspirations in the new room.

## Best Practices

### For Agents

1. **Record decisions** — When you make an architectural choice, log it
2. **Capture questions** — If something is unclear, note it for humans
3. **Mark milestones** — Help track progress through long tasks
4. **Add ideas** — Park good ideas that aren't in scope right now

### Decision Format
```
/decide Using [technology] for [purpose] because [rationale]
```

Example:
```
/decide Using Redis for session storage because it handles TTL natively
```

### Milestone Format
```
/milestone [component/feature] [status]
```

Example:
```
/milestone Auth flow complete and tested
```

## Querying Patterns

### Get All Decisions
```json
journal_read({"room": "workshop", "kind": "decision"})
```

### Recent Activity
```json
journal_read({"room": "workshop", "limit": 10})
```

### Open Questions
```json
journal_read({"room": "workshop", "kind": "question"})
```

## Room Context

Journal entries are included in `room_context()` output, giving agents immediate access to project history when joining a room.

## See Also

- `/help room` — Room navigation and vibes
- `/help tools` — Full MCP reference
