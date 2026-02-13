//! Command interpreter: parsing and dispatch

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

    if let Some(rest) = input.strip_prefix('/') {
        let (name, args) = match rest.find(' ') {
            Some(pos) => (rest[..pos].to_string(), rest[pos + 1..].to_string()),
            None => (rest.to_string(), String::new()),
        };
        Input::Command { name, args }
    } else if let Some(rest) = input.strip_prefix('@') {
        let (model, message) = match rest.find(' ') {
            Some(pos) => (rest[..pos].to_string(), rest[pos + 1..].to_string()),
            None => (rest.to_string(), String::new()),
        };
        Input::Mention { model, message }
    } else {
        Input::Chat(input.to_string())
    }
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
}
