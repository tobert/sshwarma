//! Tab completion system for the SSH REPL
//!
//! Provides context-aware completions for:
//! - Commands (/rooms, /join, etc.)
//! - Room names (for /join)
//! - Model names (for @mentions)
//! - MCP tool names (for /run)
//! - Artifact IDs

mod commands;
mod models;
mod rooms;
mod tools;

use std::sync::Arc;

use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

use crate::state::SharedState;

pub use commands::CommandCompleter;
pub use models::ModelCompleter;
pub use rooms::RoomCompleter;
pub use tools::ToolCompleter;

/// A completion candidate
#[derive(Debug, Clone)]
pub struct Completion {
    /// The text to insert
    pub text: String,
    /// Display label (may include description)
    pub label: String,
    /// Fuzzy match score (higher = better match)
    pub score: u32,
}

/// Context for completion request
#[derive(Debug)]
pub struct CompletionContext<'a> {
    /// Full input line
    pub line: &'a str,
    /// Cursor position in line
    pub cursor: usize,
    /// Current room (if any)
    pub room: Option<&'a str>,
}

impl<'a> CompletionContext<'a> {
    /// Get the word being completed (from last space/special char to cursor)
    pub fn current_word(&self) -> &str {
        let before_cursor = &self.line[..self.cursor];

        // Find start of current word
        let start = before_cursor
            .rfind(|c: char| c.is_whitespace() || c == '/' || c == '@')
            .map(|i| i + 1)
            .unwrap_or(0);

        &before_cursor[start..]
    }

    /// Get the prefix including trigger char (/, @)
    pub fn triggered_word(&self) -> &str {
        let before_cursor = &self.line[..self.cursor];

        // Find start - keep the trigger char
        let start = before_cursor
            .rfind(|c: char| c.is_whitespace())
            .map(|i| i + 1)
            .unwrap_or(0);

        &before_cursor[start..]
    }

    /// Check if we're completing a command (starts with /)
    pub fn is_command(&self) -> bool {
        self.line.trim_start().starts_with('/')
    }

    /// Check if we're completing a model mention (starts with @)
    pub fn is_mention(&self) -> bool {
        let word = self.triggered_word();
        word.starts_with('@')
    }

    /// Get the command name if we're completing command arguments
    pub fn command_name(&self) -> Option<&str> {
        if !self.is_command() {
            return None;
        }

        let trimmed = self.line.trim_start();
        let cmd_end = trimmed[1..] // skip /
            .find(char::is_whitespace)
            .map(|i| i + 1)
            .unwrap_or(trimmed.len());

        Some(&trimmed[1..cmd_end])
    }
}

/// Completion engine that aggregates multiple providers
pub struct CompletionEngine {
    state: Arc<SharedState>,
    matcher: Matcher,
}

impl CompletionEngine {
    pub fn new(state: Arc<SharedState>) -> Self {
        Self {
            state,
            matcher: Matcher::new(Config::DEFAULT),
        }
    }

    /// Get completions for the current input
    pub async fn complete(&mut self, ctx: &CompletionContext<'_>) -> Vec<Completion> {
        let mut completions = Vec::new();

        // Determine what to complete based on context
        if ctx.is_mention() {
            // @model completion
            let partial = &ctx.triggered_word()[1..]; // skip @
            let models = ModelCompleter::complete(&self.state).await;
            self.filter_and_score(&mut completions, models, partial);
        } else if ctx.is_command() {
            if let Some(cmd) = ctx.command_name() {
                // Command argument completion
                match cmd {
                    "join" | "j" => {
                        let partial = ctx.current_word();
                        let rooms = RoomCompleter::complete(&self.state).await;
                        self.filter_and_score(&mut completions, rooms, partial);
                    }
                    "run" => {
                        let partial = ctx.current_word();
                        let tools = ToolCompleter::complete(&self.state).await;
                        self.filter_and_score(&mut completions, tools, partial);
                    }
                    _ => {
                        // Unknown command, no arg completion yet
                    }
                }
            } else {
                // Command name completion
                let partial = &ctx.triggered_word()[1..]; // skip /
                let commands = CommandCompleter::complete();
                self.filter_and_score(&mut completions, commands, partial);
            }
        }

        // Sort by score descending
        completions.sort_by(|a, b| b.score.cmp(&a.score));
        completions
    }

    /// Filter completions by fuzzy matching and score them
    fn filter_and_score(
        &mut self,
        out: &mut Vec<Completion>,
        candidates: Vec<Completion>,
        pattern: &str,
    ) {
        if pattern.is_empty() {
            // No filter, return all with default score
            out.extend(candidates);
            return;
        }

        let pat = Pattern::parse(pattern, CaseMatching::Smart, Normalization::Smart);
        let mut buf = Vec::new();

        for mut candidate in candidates {
            let haystack = Utf32Str::new(&candidate.text, &mut buf);
            if let Some(score) = pat.score(haystack, &mut self.matcher) {
                candidate.score = score;
                out.push(candidate);
            }
        }
    }

    /// Get the range to replace when inserting completion
    pub fn replacement_range(&self, ctx: &CompletionContext<'_>) -> (usize, usize) {
        let before_cursor = &ctx.line[..ctx.cursor];

        // Find start of word to replace
        let start = if ctx.is_mention() {
            // Include the @
            before_cursor
                .rfind('@')
                .unwrap_or(ctx.cursor)
        } else if ctx.is_command() && ctx.command_name().is_none() {
            // Include the /
            before_cursor
                .rfind('/')
                .unwrap_or(ctx.cursor)
        } else {
            // Just the current word
            before_cursor
                .rfind(char::is_whitespace)
                .map(|i| i + 1)
                .unwrap_or(0)
        };

        (start, ctx.cursor)
    }
}
