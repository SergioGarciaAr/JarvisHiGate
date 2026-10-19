//! Explicit parser for `jarvis <subcomando> [opciones]`.
//!
//! Never invokes a shell: input is tokenized here and mapped onto the typed
//! [`Command`] enum, or rejected with a [`ParseError`] that explains why.

use serde::Serialize;
use thiserror::Error;

use super::types::{Command, ConversaMode, DiagnosticoScope, PiensaMode, Subcommand};

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ParseError {
    #[error("La entrada no empieza por `jarvis`.")]
    NotACommand,
    #[error("Comillas sin cerrar en la entrada.")]
    UnterminatedQuote,
    #[error("Comando desconocido: `{0}`.")]
    UnknownSubcommand(String),
    #[error("La opción `{flag}` no existe para `{sub}`.")]
    UnknownFlag { sub: &'static str, flag: String },
    #[error("La opción `{flag}` pertenece a `{owner}`, no a `{sub}`.")]
    FlagForOtherSubcommand { sub: &'static str, flag: String, owner: &'static str },
    #[error("La opción `{0}` está repetida.")]
    DuplicateFlag(String),
    #[error("Las opciones {a} y {b} son incompatibles; elige solo una.")]
    ConflictingFlags { a: String, b: String },
    #[error("Falta indicar el modo de `{sub}`.")]
    MissingMode { sub: &'static str },
    #[error("La opción `{0}` necesita un valor.")]
    MissingValue(String),
    #[error("La opción `{0}` no acepta valor.")]
    UnexpectedValue(String),
    #[error("Argumento inesperado para `{sub}`: `{arg}`.")]
    UnexpectedArgument { sub: &'static str, arg: String },
    #[error("Falta el tema sobre el que pensar.")]
    MissingPrompt,
}

impl ParseError {
    /// Short usage hint shown under the error.
    pub fn hint(&self) -> String {
        match self {
            ParseError::NotACommand => "Los comandos empiezan por `jarvis`, por ejemplo `jarvis ayuda`.".into(),
            ParseError::UnterminatedQuote => "Cierra las comillas: `jarvis claw --task \"texto\"`.".into(),
            ParseError::UnknownSubcommand(s) => match suggest(s, Subcommand::ALL.iter().map(|s| s.name())) {
                Some(best) => format!("¿Quisiste decir `jarvis {best}`? Usa `jarvis ayuda` para ver todos."),
                None => "Comandos disponibles: piensa, diagnostico, conversa, claw, ayuda.".into(),
            },
            ParseError::UnknownFlag { sub, flag } => {
                let sc = Subcommand::from_token(sub).expect("valid subcommand");
                match suggest(flag, sc.flags().iter().copied()) {
                    Some(best) => format!("¿Quisiste decir `{best}`? Ver `jarvis {sub} --help`."),
                    None => format!("Ver `jarvis {sub} --help`."),
                }
            }
            ParseError::FlagForOtherSubcommand { flag, owner, .. } => format!("Prueba `jarvis {owner} {flag}`."),
            ParseError::DuplicateFlag(_) | ParseError::UnexpectedValue(_) => "Revisa la sintaxis con `jarvis ayuda`.".into(),
            ParseError::ConflictingFlags { .. } | ParseError::MissingMode { .. } | ParseError::UnexpectedArgument { .. } => {
                format!("Ver `jarvis {} --help`.", self.subcommand_name().unwrap_or("ayuda"))
            }
            ParseError::MissingValue(f) => format!("Ejemplo: `jarvis claw {f} \"Analiza este repositorio\"`."),
            ParseError::MissingPrompt => "Ejemplo: `jarvis piensa --plan organizar mi semana`.".into(),
        }
    }

    fn subcommand_name(&self) -> Option<&'static str> {
        match self {
            ParseError::MissingMode { sub } | ParseError::UnexpectedArgument { sub, .. } => Some(sub),
            ParseError::ConflictingFlags { a, .. } => Subcommand::ALL
                .iter()
                .find(|s| s.flags().contains(&a.as_str()))
                .map(|s| s.name()),
            _ => None,
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            ParseError::NotACommand => "not_a_command",
            ParseError::UnterminatedQuote => "unterminated_quote",
            ParseError::UnknownSubcommand(_) => "unknown_subcommand",
            ParseError::UnknownFlag { .. } => "unknown_flag",
            ParseError::FlagForOtherSubcommand { .. } => "flag_for_other_subcommand",
            ParseError::DuplicateFlag(_) => "duplicate_flag",
            ParseError::ConflictingFlags { .. } => "conflicting_flags",
            ParseError::MissingMode { .. } => "missing_mode",
            ParseError::MissingValue(_) => "missing_value",
            ParseError::UnexpectedValue(_) => "unexpected_value",
            ParseError::UnexpectedArgument { .. } => "unexpected_argument",
            ParseError::MissingPrompt => "missing_prompt",
        }
    }

    pub fn info(&self) -> ParseErrorInfo {
        ParseErrorInfo { kind: self.kind(), message: self.to_string(), hint: self.hint() }
    }
}

/// Serializable form sent to the UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ParseErrorInfo {
    pub kind: &'static str,
    pub message: String,
    pub hint: String,
}

/// Splits input into tokens, honouring single, double and typographic quotes
/// (macOS replaces `"` with `“ ”` when smart quotes are on).
pub fn tokenize(input: &str) -> Result<Vec<String>, ParseError> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_token = false;
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' | '\'' | '“' | '”' | '‘' | '’' => {
                let close: &[char] = match c {
                    '"' => &['"'],
                    '\'' => &['\''],
                    '“' | '”' => &['”', '“'],
                    _ => &['’', '‘'],
                };
                in_token = true;
                let mut closed = false;
                while let Some(q) = chars.next() {
                    if close.contains(&q) {
                        closed = true;
                        break;
                    }
                    if q == '\\' && c == '"'
                        && let Some(&next) = chars.peek()
                            && (next == '"' || next == '\\') {
                                current.push(next);
                                chars.next();
                                continue;
                            }
                    current.push(q);
                }
                if !closed {
                    return Err(ParseError::UnterminatedQuote);
                }
            }
            c if c.is_whitespace() => {
                if in_token {
                    tokens.push(std::mem::take(&mut current));
                    in_token = false;
                }
            }
            c => {
                in_token = true;
                current.push(c);
            }
        }
    }
    if in_token {
        tokens.push(current);
    }
    Ok(tokens)
}

/// True if the input is addressed to the command layer (`jarvis …`).
pub fn is_command(input: &str) -> bool {
    let trimmed = input.trim_start();
    let first = trimmed.split_whitespace().next().unwrap_or("");
    first.eq_ignore_ascii_case("jarvis")
}

pub fn parse(input: &str) -> Result<Command, ParseError> {
    if !is_command(input) {
        return Err(ParseError::NotACommand);
    }
    let tokens = tokenize(input)?;
    let mut rest = tokens.into_iter().skip(1);

    let Some(sub_token) = rest.next() else {
        return Ok(Command::Ayuda { topic: None });
    };
    if sub_token == "--help" || sub_token == "-h" {
        return Ok(Command::Ayuda { topic: None });
    }
    let Some(sub) = Subcommand::from_token(&sub_token) else {
        return Err(ParseError::UnknownSubcommand(sub_token));
    };

    let args: Vec<String> = rest.collect();

    if sub == Subcommand::Ayuda {
        return match args.as_slice() {
            [] => Ok(Command::Ayuda { topic: None }),
            [topic] => match Subcommand::from_token(topic) {
                Some(t) => Ok(Command::Ayuda { topic: Some(t) }),
                None => Err(ParseError::UnknownSubcommand(topic.clone())),
            },
            [_, extra, ..] => Err(ParseError::UnexpectedArgument { sub: sub.name(), arg: extra.clone() }),
        };
    }

    let parsed = collect_args(sub, args)?;
    if parsed.help {
        return Ok(Command::Ayuda { topic: Some(sub) });
    }
    build(sub, parsed)
}

#[derive(Default)]
struct ParsedArgs {
    flags: Vec<String>,
    task: Option<String>,
    positionals: Vec<String>,
    help: bool,
}

fn collect_args(sub: Subcommand, args: Vec<String>) -> Result<ParsedArgs, ParseError> {
    let mut out = ParsedArgs::default();
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        if arg == "--help" || arg == "-h" {
            out.help = true;
            continue;
        }
        if !arg.starts_with("--") || arg == "--" {
            out.positionals.push(arg);
            continue;
        }
        let (name, inline_value) = match arg.split_once('=') {
            Some((n, v)) => (n.to_lowercase(), Some(v.to_string())),
            None => (arg.to_lowercase(), None),
        };
        if !sub.flags().contains(&name.as_str()) {
            if let Some(owner) = Subcommand::ALL.iter().find(|s| s.flags().contains(&name.as_str())) {
                return Err(ParseError::FlagForOtherSubcommand { sub: sub.name(), flag: name, owner: owner.name() });
            }
            return Err(ParseError::UnknownFlag { sub: sub.name(), flag: name });
        }
        if out.flags.contains(&name) {
            return Err(ParseError::DuplicateFlag(name));
        }
        if name == "--task" {
            let value = match inline_value {
                Some(v) => v,
                None => match iter.next() {
                    Some(v) if !v.starts_with("--") => v,
                    _ => return Err(ParseError::MissingValue(name)),
                },
            };
            if value.trim().is_empty() {
                return Err(ParseError::MissingValue(name));
            }
            out.task = Some(value);
        } else if inline_value.is_some() {
            return Err(ParseError::UnexpectedValue(name));
        }
        out.flags.push(name);
    }
    Ok(out)
}

/// Picks the single mode flag among `flags`, rejecting combinations.
fn single_mode(flags: &[String]) -> Result<Option<&str>, ParseError> {
    match flags {
        [] => Ok(None),
        [one] => Ok(Some(one.as_str())),
        [a, b, ..] => Err(ParseError::ConflictingFlags { a: a.clone(), b: b.clone() }),
    }
}

fn no_positionals(sub: Subcommand, p: &[String]) -> Result<(), ParseError> {
    match p.first() {
        Some(arg) => Err(ParseError::UnexpectedArgument { sub: sub.name(), arg: arg.clone() }),
        None => Ok(()),
    }
}

fn build(sub: Subcommand, a: ParsedArgs) -> Result<Command, ParseError> {
    match sub {
        Subcommand::Piensa => {
            let mode = match single_mode(&a.flags)? {
                Some("--plan") => PiensaMode::Plan,
                Some("--comparar") => PiensaMode::Comparar,
                _ => return Err(ParseError::MissingMode { sub: sub.name() }),
            };
            let prompt = a.positionals.join(" ").trim().to_string();
            if prompt.is_empty() {
                return Err(ParseError::MissingPrompt);
            }
            Ok(Command::Piensa { mode, prompt })
        }
        Subcommand::Diagnostico => {
            no_positionals(sub, &a.positionals)?;
            let scope = match single_mode(&a.flags)? {
                Some("--red") => DiagnosticoScope::Red,
                Some("--ram") => DiagnosticoScope::Ram,
                Some("--full") => DiagnosticoScope::Full,
                _ => DiagnosticoScope::General,
            };
            Ok(Command::Diagnostico { scope })
        }
        Subcommand::Conversa => {
            no_positionals(sub, &a.positionals)?;
            let mode = match single_mode(&a.flags)? {
                Some("--manos-libres") => ConversaMode::ManosLibres,
                _ => ConversaMode::Voz,
            };
            Ok(Command::Conversa { mode })
        }
        Subcommand::Claw => {
            no_positionals(sub, &a.positionals)?;
            match a.task {
                Some(task) => Ok(Command::Claw { task }),
                None => Err(ParseError::MissingValue("--task".into())),
            }
        }
        Subcommand::Ayuda => unreachable!("handled before collect_args"),
    }
}

/// Closest candidate within an edit distance of 3, for "¿quisiste decir…?".
fn suggest<'a>(input: &str, candidates: impl Iterator<Item = &'a str>) -> Option<&'a str> {
    let input = input.to_lowercase();
    candidates
        .map(|c| (levenshtein(&input, c), c))
        .filter(|(d, _)| *d <= 3)
        .min_by_key(|(d, _)| *d)
        .map(|(_, c)| c)
}

fn levenshtein(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut cur = vec![i + 1; b.len() + 1];
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != *cb);
            cur[j + 1] = (prev[j] + cost).min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        prev = cur;
    }
    prev[b.len()]
}
