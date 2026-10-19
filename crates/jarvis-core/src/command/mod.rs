//! Jarvis Command Layer: tokenizer, parser, help and typed commands.

mod help;
mod parser;
mod types;

pub use help::{Availability, availability, help_text};
pub use parser::{ParseError, ParseErrorInfo, is_command, parse, tokenize};
pub use types::{Command, ConversaMode, DiagnosticoScope, PiensaMode, Subcommand};
