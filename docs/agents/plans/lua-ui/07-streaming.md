# 07: Model Streaming

**File:** `src/ssh/streaming.rs`, `src/lua/tools.rs`
**Focus:** Stream model responses as rows, notify Lua
**Dependencies:** 02-input, 04-tools-api
**Unblocks:** 08-integration

---

## Task

Implement @mention handling and model response streaming. Rust writes chunks as rows, notifies Lua via callback.

**Why this task?** @mentions are core functionality. Streaming must work with the new row-based rendering.

**Deliverables:**
1. Parse @mention from input (in Lua, delegates to Rust for model call)
2. Rust streams response chunks as rows
3. Lua callback `on_row_added` triggered per row
4. Tool calls during response also become rows
5. Streaming state tracked for UI indicator

**Definition of Done:**
```bash
cargo fmt --check
cargo clippy -- -D warnings
cargo check
cargo test
```

## Out of Scope

- Chat rendering — that's 06-chat
- Model selection — existing functionality, keep as-is
- Tool execution — existing functionality, just ensure rows created

Focus ONLY on the streaming pipeline from @mention to rows.

---

## Flow

```
User types: @qwen-8b what tools do you have?
    │
    ▼
Lua input.lua parses @mention
    │
    ▼
Lua calls tools.mention(model, message)
    │
    ▼
Rust starts model stream
    │
    ├──► Each chunk: insert row (content_method = "message.model.chunk")
    │    └──► Call lua.on_row_added(buffer_id, row_id)
    │
    ├──► Tool call: insert row (content_method = "tool.call")
    │    └──► Execute tool
    │    └──► Insert row (content_method = "tool.result")
    │
    └──► Complete: insert final row (content_method = "message.model")
         └──► Call lua.on_row_added(buffer_id, row_id)
```

---

## Lua Side

```lua
-- In input handling (02-input)

function handle_mention(input)
    -- Parse @model message
    local model, message = input:match("^@(%S+)%s+(.*)")
    if not model then return end

    -- Add user message as row
    tools.add_row({
        content_method = "message.user",
        author = tools.session().username,
        content = input,
    })

    -- Start model stream (async, Rust handles)
    tools.mention(model, message)
end

-- Callback when rows are added (registered in init)
function on_row_added(buffer_id, row)
    -- Just mark chat dirty, rendering will pick up new rows
    tools.mark_dirty('chat')

    -- If streaming complete, could do additional work
    if row.content_method == "message.model" then
        -- Streaming finished
    end
end

-- Register callback
tools.on_row_added(on_row_added)
```

---

## Rust Side

```rust
// In src/lua/tools.rs

// tools.mention(model, message)
let mention_fn = {
    let state = state.clone();
    let lua_runtime = lua_runtime.clone();
    lua.create_function(move |_lua, (model, message): (String, String)| {
        // Spawn async task to handle streaming
        let state = state.clone();
        let lua_runtime = lua_runtime.clone();

        tokio::spawn(async move {
            if let Err(e) = stream_model_response(&state, &lua_runtime, &model, &message).await {
                tracing::error!("Mention error: {}", e);
                // Insert error row
                if let Err(e) = insert_error_row(&state, &e.to_string()) {
                    tracing::error!("Failed to insert error row: {}", e);
                }
            }
        });

        Ok(())
    })?
};
tools.set("mention", mention_fn)?;

// Streaming implementation
async fn stream_model_response(
    state: &SharedState,
    lua_runtime: &Arc<TokioMutex<LuaRuntime>>,
    model: &str,
    message: &str,
) -> Result<()> {
    let session = get_session_context(state)?;
    let room = session.room_name.as_ref().context("Not in a room")?;
    let buffer_id = get_room_buffer(state, room)?;

    // Get model handle
    let model_handle = state.models.get(model).context("Model not found")?;

    // Start streaming
    let mut stream = model_handle.stream(message, /* tools, history */).await?;

    let mut accumulated = String::new();
    let mut chunk_row_id: Option<String> = None;

    while let Some(chunk) = stream.next().await {
        match chunk {
            StreamChunk::Text(text) => {
                accumulated.push_str(&text);

                // Update or insert chunk row
                if let Some(ref row_id) = chunk_row_id {
                    // Update existing chunk row
                    state.db.update_row_content(row_id, &accumulated)?;
                } else {
                    // Insert new chunk row
                    let row = state.db.insert_row(buffer_id, RowInsert {
                        content_method: "message.model.chunk".to_string(),
                        author: Some(model.to_string()),
                        content: accumulated.clone(),
                        ..Default::default()
                    })?;
                    chunk_row_id = Some(row.id.clone());

                    // Notify Lua
                    notify_lua_row_added(lua_runtime, buffer_id, &row).await;
                }

                // Notify Lua of update
                notify_lua_dirty(lua_runtime, "chat").await;
            }
            StreamChunk::ToolCall { name, args } => {
                // Insert tool call row
                let tool_row = state.db.insert_row(buffer_id, RowInsert {
                    content_method: "tool.call".to_string(),
                    author: Some(model.to_string()),
                    tool_name: Some(name.clone()),
                    content: serde_json::to_string(&args)?,
                    ..Default::default()
                })?;
                notify_lua_row_added(lua_runtime, buffer_id, &tool_row).await;

                // Execute tool
                let result = execute_tool(state, &name, args).await;

                // Insert result row
                let result_row = state.db.insert_row(buffer_id, RowInsert {
                    content_method: "tool.result".to_string(),
                    author: Some(name.clone()),
                    content: result,
                    parent_id: Some(tool_row.id.clone()),
                    ..Default::default()
                })?;
                notify_lua_row_added(lua_runtime, buffer_id, &result_row).await;
            }
            StreamChunk::Done => break,
        }
    }

    // Convert chunk to final message
    if let Some(chunk_id) = chunk_row_id {
        state.db.update_row(&chunk_id, |row| {
            row.content_method = "message.model".to_string();
        })?;
        notify_lua_dirty(lua_runtime, "chat").await;
    }

    Ok(())
}

// Notify Lua of new row
async fn notify_lua_row_added(
    lua_runtime: &Arc<TokioMutex<LuaRuntime>>,
    buffer_id: &str,
    row: &Row,
) {
    let lua = lua_runtime.lock().await;
    if let Err(e) = lua.call_on_row_added(buffer_id, row) {
        tracing::debug!("on_row_added callback error: {}", e);
    }
}

// Mark region dirty
async fn notify_lua_dirty(lua_runtime: &Arc<TokioMutex<LuaRuntime>>, region: &str) {
    let lua = lua_runtime.lock().await;
    lua.tool_state().mark_dirty(region);
}
```

---

## Row Types

```rust
// Row content_method values for streaming

const MESSAGE_USER: &str = "message.user";
const MESSAGE_MODEL: &str = "message.model";
const MESSAGE_MODEL_CHUNK: &str = "message.model.chunk";
const MESSAGE_SYSTEM: &str = "message.system";
const TOOL_CALL: &str = "tool.call";
const TOOL_RESULT: &str = "tool.result";
```

---

## Callback Registration

```rust
// In LuaRuntime

pub fn register_row_callback(&self) -> Result<()> {
    // Store callback in registry for later invocation
    let on_row_added: Function = self.lua.globals().get("on_row_added")?;
    self.lua.set_named_registry_value("on_row_added", on_row_added)?;
    Ok(())
}

pub fn call_on_row_added(&self, buffer_id: &str, row: &Row) -> Result<()> {
    let callback: Function = self.lua.named_registry_value("on_row_added")?;

    let row_table = self.lua.create_table()?;
    row_table.set("id", row.id.clone())?;
    row_table.set("content_method", row.content_method.clone())?;
    row_table.set("author", row.author.clone())?;
    row_table.set("content", row.content.clone())?;
    // ... other fields

    callback.call::<()>((buffer_id, row_table))?;
    Ok(())
}
```

---

## Acceptance Criteria

- [ ] `@model message` starts streaming
- [ ] Chunks appear incrementally in chat
- [ ] Streaming indicator shows during response
- [ ] Tool calls create separate rows
- [ ] Tool results linked to tool calls
- [ ] Final message row has correct content_method
- [ ] Multiple concurrent @mentions work
- [ ] Errors display as system messages
- [ ] on_row_added callback invoked per row
- [ ] Chat region marked dirty on updates
- [ ] History includes all rows after completion
