# UI Rebuild Review Findings

## Executive Summary

The transition from a Rust-based layout system to a pure Lua bar/page system is a significant architectural improvement. It successfully decouples layout logic from the core engine, enabling faster iteration and a more flexible UI. The implementation of Vim-style modes and ephemeral pages is robust and idiomatic.

## 1. Code Correctness & Logic

*   **Layout Module (`layout.lua`)**:
    *   **Constraint Solver**: The logic is sound. The pervasive use of `math.max(0, ...)` correctly handles edge cases, ensuring that negative dimensions (which could cause panics or rendering artifacts) are impossible, even on tiny terminals.
    *   **Rect Operations**: `split_vertical`, `split_horizontal`, and `sub` correctly implement the geometry logic required for a tiling/stacking layout.

*   **Bars Module (`bars.lua`)**:
    *   **Spacer Distribution**: The algorithm `spacer_width + (i <= extra and 1 or 0)` is the correct, standard way to distribute remainder pixels. This ensures a pixel-perfect layout without gaps or "jitter" when resizing.
    *   **Stacking Logic**: The priority-based stacking from edges inward is implemented correctly in `layout.compute_bars`.

*   **Scroll Module (`scroll.lua`)**:
    *   **Following Behavior**: The logic correctly differentiates between "user scrolled up" (detach) and "at bottom" (follow). Re-attaching on `to_bottom` is handled correctly.
    *   **Range Calculation**: `visible_range()` returns a half-open interval `[start, end)`, which is the correct convention for loop iterators in Lua/Rust interactions.

*   **Input Module (`input.lua`)**:
    *   **UTF-8 Handling**: The cursor logic correctly handles multi-byte characters. The 0-indexed cursor (Rust style) vs 1-indexed string operations (Lua style) impedance mismatch is handled correctly (e.g., `prev_utf8_start` logic).

## 2. API Design & Architecture

*   **Decoupling**: The separation of concerns is excellent:
    *   `input.lua`: Low-level byte parsing and buffer manipulation.
    *   `mode.lua`: Key bindings and policy (Normal vs Insert).
    *   `screen.lua`: High-level composition of the specific UI.
    This makes the system highly extensible.

*   **Vim Modes**: The explicit state machine (`M.current = "normal" | "insert"`) is superior to the previous heuristic-based approach. It eliminates ambiguity about how a key press will be interpreted.

*   **Naming**:
    *   **Minor Inconsistency**: "send" (Lua action) vs "execute" (Rust enum). `mod.rs` handles both, so it's not a bug, but standardizing on one (e.g., `execute`) in `mode.lua` would be cleaner long-term.

## 3. Specific Concerns Addressed

*   **Memory Leaks**: `pages.lua` is safe. Page state is stored in a simple table and managed via explicit open/close operations. Since `chat` (index 1) is never removed, the base state is stable.
*   **Race Conditions**: Lua runs single-threaded within the Rust host. `on_input` processes events sequentially. There is no risk of race conditions between mode switching and input buffer modification.
*   **Performance**: The Lua layout computation is extremely lightweight (simple arithmetic on small tables). It will not be a bottleneck compared to the actual terminal I/O or LLM inference.

## 4. Recommendations

### Missing Functionality
*   **Vim Bindings**: While `hjkl` are present, the Normal mode lacks some standard navigation keys that users might expect:
    *   `0` (Start of line)
    *   `$` (End of line)
    *   `w` / `b` (Next/Prev word)
    *   `Ctrl+U` / `Ctrl+D` (Half-page up/down) - currently `PageUp`/`PageDown` keys are mapped, but the Ctrl shortcuts are iconic.

### Cleanup
*   **Rust Code Removal**: The files `src/ui/layout.rs` and `src/ui/scroll.rs` (if fully superseded) should be removed or deprecated to reduce binary size and confusion.
*   **Tab Completion**: The `tab` action is emitted by `mode.lua` but currently appears to be a no-op in the UI code reviewed. Ensure the Rust side has a handler for `InputAction::Tab` or implement the completion UI in Lua.

### Testing
*   **Tiny Terminal**: Add a test case for a 1x1 or 2x2 terminal size to `layout.lua` tests. This is the most common source of "attempt to perform arithmetic on nil" or index-out-of-bounds errors in TUI layout engines.
*   **Bar Priority**: Add a test verifying that bars with higher priority correctly stack closer to the edge than lower priority ones.

## Conclusion

The new UI system is solid. It moves complexity from rigid Rust structures to flexible Lua scripts, which is exactly the right trade-off for a customizable TUI application. The code is clean, readable, and idiomatic.

**Status**: **Approved** (with recommendation to delete old Rust code).
