# sshwarma TODO

## Equipment System

The `/equip` system exists but may have gaps:

### Clarity needed

- **User vs room equipment**: `/equip` binds to user session, `/bring` binds assets to room. Is this distinction clear to users? Should equipment be room-scoped instead?

- **Model inheritance**: When @mentioning a model, does it see/use the user's equipped tools? Need to verify wrap() includes equipped tools in model context.

- **Persistence**: Is equipment persisted across SSH sessions? Verify db schema and session restoration.

### Potential issues

- **Tool filtering**: When a model calls tools, are non-equipped tools actually blocked? Or just hidden from context?

- **Qualified names**: `/equip holler:sample` assumes `server:tool` format. What about internal tools? Collisions?

- **Feedback**: `/inv` shows `+` for available, `o` for unavailable. Is this clear? What makes a tool unavailable?

### Missing features

- **Bulk operations**: No `/equip all` or `/unequip all`
- **Defaults**: No way to set default equipment for new sessions
- **Room templates**: Can't define "this room comes with these tools equipped"

## Journals → Prompts Migration

Current `/note`, `/decide`, `/idea`, `/milestone` may be replaced. The prompt system (`/prompt`) exists but relationship to journals unclear.

## Context Composition (wrap)

- Verify equipped tools appear in model context
- Verify room-bound assets appear in context
- Token budget management for large equipment lists

## UI

- Screenshots needed for README
- Status bar could show equipped tool count
