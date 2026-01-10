# Rooms — Quick Reference

Rooms are shared spaces where users and models collaborate. Think MUD meets workspace.

## Core Concepts

### The Metaphor
- **Lobby**: Landing zone on connect, list and join rooms
- **Rooms**: Named spaces with vibes, assets, and exits
- **Exits**: Connections between rooms (like MUD directions)
- **Vibes**: Atmosphere description that shapes interactions

## Navigation

### SSH Commands
```
/rooms              List all rooms
/join <name>        Enter a room
/leave              Return to lobby
/go <direction>     Follow an exit
/exits              Show available exits
/look               Room summary
```

### MCP Tools
```json
list_rooms()                    // List all rooms
create_room(name, description)  // Create new room
room_context(room)              // Get full context
add_exit(room, direction, target, bidirectional)
```

## Room Context

When entering a room, get oriented:
1. **Vibe** — What's the atmosphere?
2. **Participants** — Who's here? (users + models)
3. **Assets** — What artifacts are bound?
4. **Exits** — Where can we go?

```json
// MCP: Get everything at once
room_context("workshop")
```

## Vibes

A vibe is a short description that sets the room's tone.

### SSH
```
/vibe                   Show current vibe
/vibe focused coding    Set new vibe
```

### MCP
```json
set_vibe("workshop", "collaborative debugging session")
```

Vibes help models understand context and tone their responses appropriately.

## Exits

Exits connect rooms with named directions.

### Creating Exits
```
/portal north garden    Create bidirectional exit
```

```json
add_exit({
  "room": "workshop",
  "direction": "north",
  "target": "garden",
  "bidirectional": true
})
```

### Navigation
```
/go north               Follow exit
/exits                  List available exits
```

Directions can be anything: `north`, `studio`, `archive`, `upstairs`.

## Forking Rooms

Fork creates a new room inheriting:
- Vibe
- Asset bindings

```
/fork workshop-v2       Fork current room
```

```json
fork_room("workshop", "workshop-v2")
```

Great for branching off an experiment without losing context.

## Model Navigation

By default, models can navigate between rooms when @mentioned. Control this per-room:

```
/nav off    Disable model navigation (keep them focused)
/nav on     Re-enable navigation
```

## Common Patterns

### Agent Onboarding
```
1. /rooms → find relevant room
2. /join <room>
3. /look → orient yourself
4. Start collaborating
```

### Creating a Session
```
1. /create my-session
2. /join my-session
3. /vibe "brainstorming new features"
4. @qwen-8b let's explore...
```

### Connecting Spaces
```
1. /join workshop
2. /portal archive reference-room
3. /go archive → jump to reference
4. /go workshop → jump back
```

## See Also

- `/help tools` — Full MCP reference
