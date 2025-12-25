//! Command interpreter: parsing and dispatch

pub mod commands;

use anyhow::Result;

/// Parsed input from user
#[derive(Debug)]
pub enum Input {
    /// /command [args]
    Command { name: String, args: String },
    /// @model message
    Mention { model: String, message: String },
    /// Plain chat text
    Chat(String),
    /// Empty input
    Empty,
}

/// Parse raw input into structured form
pub fn parse(input: &str) -> Input {
    let input = input.trim();

    if input.is_empty() {
        return Input::Empty;
    }

    if input.starts_with('/') {
        let rest = &input[1..];
        let (name, args) = match rest.find(' ') {
            Some(pos) => (rest[..pos].to_string(), rest[pos + 1..].to_string()),
            None => (rest.to_string(), String::new()),
        };
        Input::Command { name, args }
    } else if input.starts_with('@') {
        let rest = &input[1..];
        let (model, message) = match rest.find(' ') {
            Some(pos) => (rest[..pos].to_string(), rest[pos + 1..].to_string()),
            None => (rest.to_string(), String::new()),
        };
        Input::Mention { model, message }
    } else {
        Input::Chat(input.to_string())
    }
}

/// Parse key=value arguments
pub fn parse_args(args: &str) -> Vec<(String, String)> {
    let mut result = Vec::new();

    for part in args.split_whitespace() {
        if let Some(eq_pos) = part.find('=') {
            let key = part[..eq_pos].to_string();
            let value = part[eq_pos + 1..].to_string();
            result.push((key, value));
        } else {
            // Positional arg, use index as key
            result.push((result.len().to_string(), part.to_string()));
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_command() {
        match parse("/join hootenanny") {
            Input::Command { name, args } => {
                assert_eq!(name, "join");
                assert_eq!(args, "hootenanny");
            }
            _ => panic!("expected command"),
        }
    }

    #[test]
    fn test_parse_mention() {
        match parse("@qwen-8b generate something cool") {
            Input::Mention { model, message } => {
                assert_eq!(model, "qwen-8b");
                assert_eq!(message, "generate something cool");
            }
            _ => panic!("expected mention"),
        }
    }

    #[test]
    fn test_parse_chat() {
        match parse("hello world") {
            Input::Chat(msg) => assert_eq!(msg, "hello world"),
            _ => panic!("expected chat"),
        }
    }

    #[test]
    fn test_parse_args() {
        let args = parse_args("temperature=1.0 bars=4");
        assert_eq!(args.len(), 2);
        assert_eq!(args[0], ("temperature".to_string(), "1.0".to_string()));
        assert_eq!(args[1], ("bars".to_string(), "4".to_string()));
    }
}
