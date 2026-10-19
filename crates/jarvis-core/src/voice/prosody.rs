//! Text preparation for speech: turns a chat answer (Markdown-ish) into
//! plain Spanish text that any engine (neural or `say`) pronounces well.
//!
//! Only pronunciation help is applied: dates, times, numbers, units,
//! acronyms, IPs, versions, domains, commands, and Markdown removal. No
//! pauses are inserted and nothing is truncated: phase 3A.2 inserted
//! `[[slnc]]` silences (also inside sentences) and cut answers at 4000
//! characters, which were the measured causes of the audible cuts.
//!
//! The text coming from the model is never trusted to contain `say`
//! commands: any `[[`/`]]` in it is removed.

#[derive(Debug, Clone, PartialEq)]
enum Block {
    Heading(String),
    Paragraph(String),
    Bullet(String),
    Numbered(u32, String),
    Code,
    TableRow(Vec<String>),
    Break,
}

fn numbered_item(line: &str) -> Option<(u32, &str)> {
    let digits: String = line.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() || digits.len() > 3 {
        return None;
    }
    let rest = &line[digits.len()..];
    let rest = rest.strip_prefix(". ").or_else(|| rest.strip_prefix(") "))?;
    Some((digits.parse().ok()?, rest))
}

fn segment(input: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut para: Vec<String> = Vec::new();
    let mut in_code = false;
    let flush = |para: &mut Vec<String>, blocks: &mut Vec<Block>| {
        if !para.is_empty() {
            blocks.push(Block::Paragraph(para.join(" ")));
            para.clear();
        }
    };
    for raw in input.lines() {
        let line = raw.trim();
        if line.starts_with("```") || line.starts_with("~~~") {
            if !in_code {
                flush(&mut para, &mut blocks);
                blocks.push(Block::Code);
            }
            in_code = !in_code;
            continue;
        }
        if in_code {
            continue;
        }
        if line.is_empty() {
            flush(&mut para, &mut blocks);
            if !matches!(blocks.last(), Some(Block::Break) | None) {
                blocks.push(Block::Break);
            }
            continue;
        }
        if line.starts_with('#') {
            flush(&mut para, &mut blocks);
            blocks.push(Block::Heading(line.trim_start_matches('#').trim().to_string()));
            continue;
        }
        if let Some(rest) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")).or_else(|| line.strip_prefix("+ ")).or_else(|| line.strip_prefix("• ")) {
            flush(&mut para, &mut blocks);
            blocks.push(Block::Bullet(rest.trim().to_string()));
            continue;
        }
        if let Some((n, rest)) = numbered_item(line) {
            flush(&mut para, &mut blocks);
            blocks.push(Block::Numbered(n, rest.trim().to_string()));
            continue;
        }
        if line.starts_with('|') && line.ends_with('|') {
            flush(&mut para, &mut blocks);
            let cells: Vec<String> = line.trim_matches('|').split('|').map(|c| c.trim().to_string()).collect();
            let separator = cells.iter().all(|c| !c.is_empty() && c.chars().all(|ch| matches!(ch, '-' | ':' | ' ')));
            if !separator {
                blocks.push(Block::TableRow(cells));
            }
            continue;
        }
        para.push(line.trim_start_matches('>').trim().to_string());
    }
    flush(&mut para, &mut blocks);
    while matches!(blocks.last(), Some(Block::Break)) {
        blocks.pop();
    }
    blocks
}

const ORDINALS: [&str; 10] = ["Primero", "Segundo", "Tercero", "Cuarto", "Quinto", "Sexto", "Séptimo", "Octavo", "Noveno", "Décimo"];

const MONTHS: [&str; 12] = [
    "enero", "febrero", "marzo", "abril", "mayo", "junio", "julio", "agosto", "septiembre", "octubre", "noviembre", "diciembre",
];

/// Whole-word replacements. Acronyms are spelled the way they are said in Spanish.
const LEXICON: &[(&str, &str)] = &[
    ("p. ej.", "por ejemplo"),
    ("p.ej.", "por ejemplo"),
    ("etc.", "etcétera"),
    ("aprox.", "aproximadamente"),
    ("máx.", "máximo"),
    ("mín.", "mínimo"),
    ("pág.", "página"),
    ("núm.", "número"),
    ("n.º", "número"),
    ("nº", "número"),
    ("EE. UU.", "Estados Unidos"),
    ("EE.UU.", "Estados Unidos"),
    ("Sr.", "señor"),
    ("Sra.", "señora"),
    ("Dr.", "doctor"),
    ("Dra.", "doctora"),
    ("Ud.", "usted"),
    ("Uds.", "ustedes"),
    ("vs.", "frente a"),
    ("vs", "frente a"),
    ("CPU", "ce pe u"),
    ("GPU", "ge pe u"),
    ("SSD", "ese ese de"),
    ("USB", "u ese be"),
    ("DNS", "de ene ese"),
    ("IP", "i pe"),
    ("IPv4", "i pe ve cuatro"),
    ("IPv6", "i pe ve seis"),
    ("TCP", "te ce pe"),
    ("UDP", "u de pe"),
    ("URL", "u erre ele"),
    ("API", "a pe i"),
    ("JSON", "yeison"),
    ("HTML", "hache te eme ele"),
    ("HTTP", "hache te te pe"),
    ("HTTPS", "hache te te pe ese"),
    ("PDF", "pe de efe"),
    ("SSH", "ese ese hache"),
    ("VPN", "uve pe ene"),
    ("CLI", "ce ele i"),
    ("LLM", "ele ele eme"),
    ("TTS", "te te ese"),
    ("STT", "ese te te"),
    ("macOS", "mac o ese"),
    ("iOS", "i o ese"),
    ("Wi-Fi", "wifi"),
];

/// Units read after a number: (symbol, singular, plural).
const UNITS: &[(&str, &str, &str)] = &[
    ("TB", "terabyte", "terabytes"),
    ("GB", "gigabyte", "gigabytes"),
    ("MB", "megabyte", "megabytes"),
    ("KB", "kilobyte", "kilobytes"),
    ("GHz", "gigahercio", "gigahercios"),
    ("MHz", "megahercio", "megahercios"),
    ("ms", "milisegundo", "milisegundos"),
    ("km/h", "kilómetro por hora", "kilómetros por hora"),
    ("km", "kilómetro", "kilómetros"),
    ("kg", "kilo", "kilos"),
    ("°C", "grado", "grados"),
    ("ºC", "grado", "grados"),
    ("mAh", "miliamperio hora", "miliamperios hora"),
    ("€", "euro", "euros"),
    ("W", "vatio", "vatios"),
];

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Replaces `from` when it stands as a whole word/token.
fn replace_token(text: &str, from: &str, to: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while let Some(pos) = text[i..].find(from) {
        let start = i + pos;
        let end = start + from.len();
        let before_ok = text[..start].chars().next_back().is_none_or(|c| !is_word_char(c));
        let first_is_word = from.chars().next().is_some_and(is_word_char);
        let last_is_word = from.chars().next_back().is_some_and(is_word_char);
        let after_ok = text[end..].chars().next().is_none_or(|c| !is_word_char(c));
        if (before_ok || !first_is_word) && (after_ok || !last_is_word) {
            out.push_str(&text[i..start]);
            out.push_str(to);
        } else {
            out.push_str(&text[i..end]);
        }
        i = end;
    }
    out.push_str(&text[i..]);
    out
}

/// Splits text into tokens separated by whitespace, keeping the separators.
fn map_tokens(text: &str, f: impl Fn(&str) -> Option<String>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut token = String::new();
    let emit = |token: &mut String, out: &mut String| {
        if token.is_empty() {
            return;
        }
        // Keep trailing punctuation outside the token transform.
        let core_end = token.trim_end_matches(['.', ',', ';', ':', '?', '!', ')', '»', '"']).len();
        let (core, tail) = token.split_at(core_end);
        let lead_len = core.len() - core.trim_start_matches(['(', '«', '"', '¿', '¡']).len();
        let (lead, core) = core.split_at(lead_len);
        match f(core) {
            Some(r) => {
                out.push_str(lead);
                out.push_str(&r);
                out.push_str(tail);
            }
            None => out.push_str(token),
        }
        token.clear();
    };
    for c in text.chars() {
        if c.is_whitespace() {
            emit(&mut token, &mut out);
            out.push(c);
        } else {
            token.push(c);
        }
    }
    emit(&mut token, &mut out);
    out
}

/// Spanish decimal comma read explicitly: "391,5" → "391 coma 5". Only one or
/// two decimals, so "1,234" (possibly an English thousands separator) is kept.
/// Measured: the neural engine misread "391,5" without this.
fn decimal(n: &str) -> String {
    match n.split_once(',') {
        Some((int, dec))
            if !int.is_empty()
                && int.chars().all(|c| c.is_ascii_digit())
                && (1..=2).contains(&dec.len())
                && dec.chars().all(|c| c.is_ascii_digit()) =>
        {
            format!("{int} coma {dec}")
        }
        _ => n.to_string(),
    }
}

fn date_words(day: u32, month: u32, year: Option<u32>) -> Option<String> {
    if !(1..=31).contains(&day) || !(1..=12).contains(&month) {
        return None;
    }
    let m = MONTHS[(month - 1) as usize];
    Some(match year {
        Some(y) => format!("{day} de {m} de {y}"),
        None => format!("{day} de {m}"),
    })
}

fn normalize_token(t: &str) -> Option<String> {
    let nums = |s: &str, sep: char| -> Option<Vec<u32>> { s.split(sep).map(|p| if p.is_empty() || p.len() > 4 { None } else { p.parse().ok() }).collect() };

    // IPv4 address: read each octet, joined with "punto".
    if t.split('.').count() == 4 && t.split('.').all(|p| !p.is_empty() && p.len() <= 3 && p.chars().all(|c| c.is_ascii_digit()))
        && let Some(v) = nums(t, '.')
            && v.iter().all(|n| *n <= 255) {
                return Some(v.iter().map(u32::to_string).collect::<Vec<_>>().join(" punto "));
            }
    // Version numbers like v1.2.3 / 1.2.3.
    let ver = t.strip_prefix('v').unwrap_or(t);
    if ver.split('.').count() == 3 && ver.split('.').all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit())) {
        let spoken = ver.split('.').collect::<Vec<_>>().join(" punto ");
        return Some(if t.starts_with('v') { format!("versión {spoken}") } else { spoken });
    }
    // Dates: dd/mm/yyyy, d/m, and ISO yyyy-mm-dd.
    if let Some(v) = nums(t, '/') {
        match v.as_slice() {
            [d, m, y] if *y >= 1000 => return date_words(*d, *m, Some(*y)),
            [d, m] if t.matches('/').count() == 1 && *d <= 31 && *m <= 12 && t.len() <= 5 => return date_words(*d, *m, None),
            _ => {}
        }
    }
    if t.len() == 10 && t.as_bytes()[4] == b'-' && t.as_bytes()[7] == b'-'
        && let Some(v) = nums(t, '-')
            && let [y, m, d] = v.as_slice() {
                return date_words(*d, *m, Some(*y));
            }
    // Times: HH:MM.
    if let Some((h, m)) = t.split_once(':')
        && (1..=2).contains(&h.len()) && m.len() == 2 && h.chars().chain(m.chars()).all(|c| c.is_ascii_digit()) {
            let (h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
            if h <= 23 && m <= 59 {
                return Some(if m == 0 { format!("{h} en punto") } else { format!("{h} y {m}") });
            }
        }
    // Bare domain names: apple.com → apple punto com (only well-known TLDs,
    // so ordinary words followed by a period are never touched).
    if let Some((name, tld)) = t.rsplit_once('.') {
        const TLDS: [&str; 12] = ["com", "org", "net", "es", "mx", "pe", "io", "dev", "ai", "app", "gov", "edu"];
        let name_ok = !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.');
        if name_ok && TLDS.contains(&tld) && name.chars().any(|c| c.is_ascii_alphabetic()) {
            return Some(format!("{} punto {tld}", name.replace('.', " punto ")));
        }
    }
    // Percentages glued to the number: 45% / 12,5%.
    if let Some(n) = t.strip_suffix('%')
        && !n.is_empty() && n.chars().all(|c| c.is_ascii_digit() || c == ',' || c == '.') {
            return Some(format!("{} por ciento", decimal(n)));
        }
    // Money: $20 → 20 dólares.
    if let Some(n) = t.strip_prefix('$')
        && !n.is_empty() && n.chars().all(|c| c.is_ascii_digit() || c == ',' || c == '.') {
            return Some(format!("{} dólares", decimal(n)));
        }
    // Number glued to a unit: 16GB, 25°C.
    for (sym, one, many) in UNITS {
        if let Some(n) = t.strip_suffix(sym)
            && !n.is_empty() && n.chars().all(|c| c.is_ascii_digit() || c == ',' || c == '.') {
                return Some(format!("{} {}", decimal(n), if n == "1" { one } else { many }));
            }
    }
    let spoken = decimal(t);
    (spoken != t).then_some(spoken)
}

/// Number followed by a separate unit token: "16 GB", "45 %".
fn normalize_units(text: &str) -> String {
    let tokens: Vec<&str> = text.split(' ').collect();
    let mut out: Vec<String> = Vec::with_capacity(tokens.len());
    let mut i = 0;
    while i < tokens.len() {
        let t = tokens[i];
        let is_num = !t.is_empty() && t.chars().all(|c| c.is_ascii_digit() || c == ',' || c == '.') && t.chars().next().is_some_and(|c| c.is_ascii_digit());
        if is_num && i + 1 < tokens.len() {
            let next = tokens[i + 1];
            let core = next.trim_end_matches(['.', ',', ';', ':', '?', '!', ')']);
            let tail = &next[core.len()..];
            let unit = if core == "%" {
                Some("por ciento".to_string())
            } else {
                UNITS.iter().find(|(s, _, _)| *s == core).map(|(_, one, many)| if t == "1" { one.to_string() } else { many.to_string() })
            };
            if let Some(u) = unit {
                out.push(format!("{} {u}{tail}", decimal(t)));
                i += 2;
                continue;
            }
        }
        out.push(t.to_string());
        i += 1;
    }
    out.join(" ")
}

/// Reads a command or flag literally but intelligibly: `--ram` → "opción ram".
fn speak_code(code: &str) -> String {
    let spoken: Vec<String> = code
        .split_whitespace()
        .map(|w| {
            if let Some(flag) = w.strip_prefix("--") {
                format!("opción {}", flag.replace('-', " "))
            } else if w.len() > 1 && w.starts_with('-') && w[1..].chars().all(|c| c.is_ascii_alphabetic()) {
                format!("opción {}", &w[1..])
            } else if w.contains('/') && !w.contains("://") {
                w.trim_matches('/').replace('/', " barra ")
            } else {
                w.replace('_', " ")
            }
        })
        .collect();
    spoken.join(" ")
}

fn host_of(url: &str) -> String {
    let rest = url.split("://").nth(1).unwrap_or(url);
    let host = rest.split(['/', '?', '#']).next().unwrap_or(rest).trim_start_matches("www.");
    host.replace('.', " punto ")
}

/// Inline Markdown and symbols → speakable text, without changing meaning.
fn inline(text: &str) -> String {
    // Never let model output carry `say` commands.
    let mut s = text.replace("[[", " ").replace("]]", " ");

    // [label](url) → label
    let mut out = String::with_capacity(s.len());
    let mut rest = s.as_str();
    while let Some(start) = rest.find('[') {
        let found = rest[start..].find("](").and_then(|mid| rest[start + mid..].find(')').map(|end| (mid, start + mid + end)));
        match found {
            Some((mid, end)) => {
                out.push_str(&rest[..start]);
                out.push_str(&rest[start + 1..start + mid]);
                rest = &rest[end + 1..];
            }
            None => break,
        }
    }
    out.push_str(rest);
    s = out;

    // `code` → spoken command/identifier.
    let mut out = String::with_capacity(s.len());
    let mut parts = s.split('`');
    if let Some(first) = parts.next() {
        out.push_str(first);
    }
    for (i, part) in parts.enumerate() {
        if i % 2 == 0 { out.push_str(&speak_code(part)) } else { out.push_str(part) }
    }
    s = out;

    // Bare URLs → "enlace a host".
    s = map_tokens(&s, |t| (t.starts_with("http://") || t.starts_with("https://")).then(|| format!("enlace a {}", host_of(t))));

    // Emphasis markers only when they wrap text (keeps snake_case intact).
    for marker in ["**", "__", "~~"] {
        s = s.replace(marker, "");
    }
    s = map_tokens(&s, |t| {
        let inner = t.strip_prefix('*').and_then(|x| x.strip_suffix('*')).or_else(|| t.strip_prefix('_').and_then(|x| x.strip_suffix('_')));
        inner.filter(|x| !x.is_empty()).map(str::to_string)
    });
    s = s.replace(" *", " ").replace("* ", " ");

    // Symbols.
    for (from, to) in [
        ("→", ", "),
        ("->", ", "),
        ("=>", ", "),
        ("←", ", "),
        ("≈", " aproximadamente "),
        ("±", " más o menos "),
        ("≥", " mayor o igual que "),
        ("≤", " menor o igual que "),
        (" & ", " y "),
        (" = ", " igual a "),
        ("…", "..."),
    ] {
        s = s.replace(from, to);
    }

    s = map_tokens(&s, normalize_token);
    s = normalize_units(&s);
    for (from, to) in LEXICON {
        s = replace_token(&s, from, to);
    }
    // Drop emoji and pictographs (warning signs are handled at block level).
    s.chars()
        .filter(|c| !matches!(*c as u32, 0x1F000..=0x1FAFF | 0x2600..=0x27BF | 0xFE0F | 0x200D))
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn ends_sentence(s: &str) -> bool {
    s.trim_end().ends_with(['.', '?', '!', ':', ';', '…'])
}

fn with_period(s: String) -> String {
    if s.is_empty() || ends_sentence(&s) { s } else { format!("{s}.") }
}

fn is_warning(raw: &str) -> bool {
    let t = raw.trim_start_matches(|c: char| !c.is_alphanumeric());
    raw.contains('⚠')
        || ["Advertencia", "Atención", "Importante", "Cuidado", "Aviso"]
            .iter()
            .any(|w| t.starts_with(w) || t.strip_prefix("**").is_some_and(|x| x.starts_with(w)))
}

/// Prepares an answer for speech. Blocks (paragraphs, headings, list items)
/// are separated by newlines so engines that synthesize by fragments can
/// split at natural boundaries; sentences inside a block are untouched.
pub fn prepare(input: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    for block in segment(input) {
        let line = match block {
            Block::Break => continue,
            Block::Heading(h) => with_period(inline(&h)),
            Block::Paragraph(p) if is_warning(&p) => with_period(inline(&p)),
            Block::Paragraph(p) => inline(&p),
            Block::Bullet(b) => with_period(inline(&b)),
            Block::Numbered(n, b) => {
                let lead = ORDINALS.get((n as usize).wrapping_sub(1)).map(|o| o.to_string()).unwrap_or_else(|| format!("Número {n}"));
                format!("{lead}, {}", with_period(inline(&b)))
            }
            Block::Code => "El código está en pantalla.".to_string(),
            Block::TableRow(cells) => with_period(cells.iter().map(|c| inline(c)).filter(|c| !c.is_empty()).collect::<Vec<_>>().join(", ")),
        };
        if !line.trim().is_empty() {
            lines.push(line);
        }
    }
    lines.join("\n")
}

/// Single-line form (blocks joined by spaces), for tests and logs.
pub fn prepare_flat(input: &str) -> String {
    prepare(input).replace('\n', " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(s: &str) -> String {
        prepare_flat(s)
    }

    #[test]
    fn short_answer_is_untouched() {
        assert_eq!(prepare("¡Claro! Te ayudo con eso."), "¡Claro! Te ayudo con eso.");
    }

    #[test]
    fn questions_keep_their_marks() {
        assert_eq!(plain("¿Quieres que lo revise ahora?"), "¿Quieres que lo revise ahora?");
    }

    #[test]
    fn blocks_become_lines_without_inserted_silences() {
        let s = prepare("Primera idea. Sigue la frase.

Segunda idea.");
        assert_eq!(s, "Primera idea. Sigue la frase.
Segunda idea.");
        assert!(!s.contains("[["));
    }

    #[test]
    fn lists_are_readable_by_ear() {
        assert_eq!(
            plain("Pasos:\n1. Abre Ajustes\n2. Elige Red\n3. Pulsa Detalles"),
            "Pasos: Primero, Abre Ajustes. Segundo, Elige Red. Tercero, Pulsa Detalles."
        );
        assert!(!prepare("1. a\n2. b").contains("[["));
        assert_eq!(plain("- memoria\n- disco\n- red"), "memoria. disco. red.");
    }

    #[test]
    fn headings_and_code_blocks() {
        let s = prepare("## Resultado
Todo bien.
```rust
fn main() {}
```
Fin.");
        assert_eq!(s, "Resultado.
Todo bien.
El código está en pantalla.
Fin.");
    }

    #[test]
    fn warnings_keep_their_words_without_symbols_or_pauses() {
        let s = prepare("Hecho.

⚠️ Advertencia: no apagues el equipo durante la actualización");
        assert_eq!(s, "Hecho.
Advertencia: no apagues el equipo durante la actualización.");
    }

    #[test]
    fn numbers_dates_times_percentages_units() {
        assert_eq!(plain("El 18/10/2026 a las 14:30."), "El 18 de octubre de 2026 a las 14 y 30.");
        assert_eq!(plain("Reunión el 2026-03-05 a las 09:00."), "Reunión el 5 de marzo de 2026 a las 9 en punto.");
        assert_eq!(plain("Usa el 45% de la CPU."), "Usa el 45 por ciento de la ce pe u.");
        assert_eq!(plain("Libre: 12,5 % del disco."), "Libre: 12 coma 5 por ciento del disco.");
        assert_eq!(plain("Quedan 391,5 GB libres."), "Quedan 391 coma 5 gigabytes libres.");
        assert_eq!(plain("Subió un 3,75%."), "Subió un 3 coma 75 por ciento.");
        assert_eq!(plain("Pesa 2,5 kg y vale 1,234."), "Pesa 2 coma 5 kilos y vale 1,234.");
        assert_eq!(plain("Tiene 16 GB de RAM y 1 TB."), "Tiene 16 gigabytes de RAM y 1 terabyte.");
        assert_eq!(plain("Latencia de 44 ms."), "Latencia de 44 milisegundos.");
        assert_eq!(plain("Hace 25°C."), "Hace 25 grados.");
        assert_eq!(plain("Cuesta $20."), "Cuesta 20 dólares.");
        // Invalid dates stay as they are.
        assert_eq!(plain("Fracción 45/99."), "Fracción 45/99.");
    }

    #[test]
    fn ips_versions_and_commands_stay_intelligible() {
        assert_eq!(plain("El router es 192.168.1.1."), "El router es 192 punto 168 punto 1 punto 1.");
        assert_eq!(plain("Ollama v0.40.6 instalado."), "Ollama versión 0 punto 40 punto 6 instalado.");
        assert_eq!(plain("Ejecuta `jarvis diagnostico --ram`."), "Ejecuta jarvis diagnostico opción ram.");
        assert_eq!(plain("Revisa `/usr/bin/say`."), "Revisa usr barra bin barra say.");
        assert_eq!(plain("El DNS resuelve apple.com y docs.rs."), "El de ene ese resuelve apple punto com y docs.rs.");
        assert_eq!(plain("Fin de la frase. Otra frase."), "Fin de la frase. Otra frase.");
    }

    #[test]
    fn abbreviations_and_acronyms() {
        assert_eq!(plain("Usa p. ej. el DNS de tu proveedor, etc."), "Usa por ejemplo el de ene ese de tu proveedor, etcétera");
        assert_eq!(plain("Funciona en macOS con Wi-Fi."), "Funciona en mac o ese con wifi.");
        // Words that merely contain an acronym are not touched.
        assert_eq!(plain("La IPv6 y la API REST."), "La i pe ve seis y la a pe i REST.");
        assert_eq!(plain("Tipo IPAD."), "Tipo IPAD.");
    }

    #[test]
    fn markdown_is_not_verbalized_and_identifiers_survive() {
        assert_eq!(plain("**Importante**: mira [la guía](https://x.y/z)."), "Importante: mira la guía.");
        assert_eq!(plain("El campo `max_retries` y snake_case."), "El campo max retries y snake_case.");
        assert_eq!(plain("Ver https://www.apple.com/es/macos"), "Ver enlace a apple punto com");
        assert_eq!(plain("| Proceso | RAM |\n|---|---|\n| Chrome | 1 GB |"), "Proceso, RAM. Chrome, 1 gigabyte.");
    }

    #[test]
    fn asides_are_not_split_by_artificial_pauses() {
        assert_eq!(prepare("El disco (el principal) está bien."), "El disco (el principal) está bien.");
    }

    #[test]
    fn model_output_cannot_inject_say_commands() {
        let s = prepare("Hola [[volm 2.0]] mundo [[inpt PHON]] adiós");
        assert!(!s.contains("[[") && !s.contains("]]"), "{s}");
    }

    #[test]
    fn meaning_is_preserved_no_words_lost() {
        let input = "Jarvis revisó la memoria y encontró que Chrome usa bastante, pero la presión es normal.";
        assert_eq!(plain(input), input);
    }

    /// Phase 3A.2 cut answers at 4000 characters, so long answers stopped
    /// mid-sentence and their conclusion was never spoken.
    #[test]
    fn long_answers_are_never_truncated() {
        let para = "Esta es una explicación larga sobre el estado del sistema, con varios detalles técnicos. ";
        let long = format!("{}

Conclusión final: todo está en orden.", para.repeat(60));
        let s = prepare(&long);
        assert!(s.chars().count() > 5000);
        assert!(s.ends_with("Conclusión final: todo está en orden."));
    }

    /// Same input as the phase 3A cleanup test, now with structure preserved.
    #[test]
    fn phase_3a_cleanup_case() {
        let s = plain("# Título\n**Hola** _mundo_, mira [la guía](https://x.y).\n```rust\nfn main() {}\n```\n- punto `uno`");
        assert_eq!(s, "Título. Hola mundo, mira la guía. El código está en pantalla. punto uno.");
    }

    #[test]
    fn empty_and_code_only() {
        assert_eq!(prepare("   \n\n "), "");
        assert_eq!(prepare("```\nls -la\n```"), "El código está en pantalla.");
    }
}
