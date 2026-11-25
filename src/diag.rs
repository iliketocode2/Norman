//! Diagnostics as they leave the interpreter (design/13 §1.2).
//!
//! Inside, every diagnostic is a line of text, `file:line:col: message`,
//! which is what the checker's report, the tests and the transcript hold.
//! This module is the one place that turns such a line into what a reader
//! sees: with its label, without the design-document citations (unless
//! `--explain` asks for them), or as a JSON object (`--format json`) with
//! the location split out and, for a type mismatch, the two types.

use serde_json::{Value as J, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// The program can't be read.
    Syntax,
    /// The program was read, and the checker rejected it.
    Type,
    /// The checker accepted it, but something is probably a mistake.
    Warning,
    /// Something went wrong while a definition or expression ran.
    RunTime,
    /// A unit test failed.
    Test,
    /// A file couldn't be loaded.
    Load,
    /// What happened as a result, such as "the program was not run".
    Note,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Syntax => "syntax",
            Kind::Type => "type",
            Kind::Warning => "warning",
            Kind::RunTime => "run-time",
            Kind::Test => "test",
            Kind::Load => "load",
            Kind::Note => "note",
        }
    }

    /// The label a text diagnostic of this kind starts with, if any.
    fn label(self) -> &'static str {
        match self {
            Kind::Syntax => "syntax error: ",
            Kind::Type => "type error: ",
            Kind::RunTime => "run-time error: ",
            Kind::Warning => "warning: ",
            _ => "",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Diagnostic {
    pub kind: Kind,
    pub file: Option<String>,
    pub line: Option<u32>,
    pub column: Option<u32>,
    /// The message without its location or label.
    pub message: String,
    /// For a type mismatch: the type that was needed, and the one found.
    pub expected: Option<String>,
    pub found: Option<String>,
}

impl Diagnostic {
    /// Read a diagnostic line. Any label already on it (`syntax error: `,
    /// `run-time error: `, `warning: `) is taken off; `kind` says what it is.
    pub fn parse(kind: Kind, text: &str) -> Diagnostic {
        let mut rest = strip_label(kind, text);
        let (mut file, mut line, mut column) = (None, None, None);
        if let Some((f, l, c, after)) = location(rest) {
            (file, line, column) = (Some(f.to_string()), Some(l), Some(c));
            rest = strip_label(kind, after);
        }
        let (expected, found) = mismatch(rest).unzip();
        Diagnostic { kind, file, line, column, message: rest.to_string(), expected, found }
    }

    /// The diagnostic as a reader sees it in a terminal.
    pub fn text(&self) -> String {
        let at = match (&self.file, self.line, self.column) {
            (Some(f), Some(l), Some(c)) => format!("{}:{}:{}: ", f, l, c),
            _ => String::new(),
        };
        format!("{}{}{}", self.kind.label(), at, self.message)
    }

    pub fn json(&self) -> J {
        let mut o = json!({
            "kind": self.kind.name(),
            "file": self.file,
            "line": self.line,
            "column": self.column,
            "message": self.message,
        });
        if let (Some(e), Some(f)) = (&self.expected, &self.found) {
            o["expected"] = json!(e);
            o["found"] = json!(f);
        }
        o
    }
}

/// `s` without `kind`'s own label. Only its own: a failed test's message
/// may begin "run-time error: ", and that's part of what it says.
fn strip_label(kind: Kind, s: &str) -> &str {
    let s = s.trim_start();
    let label = kind.label();
    if label.is_empty() { s } else { s.strip_prefix(label).unwrap_or(s) }
}

/// `file:line:col: rest`, where the file name has no `: ` in it.
fn location(s: &str) -> Option<(&str, u32, u32, &str)> {
    let (head, rest) = s.split_once(": ")?;
    let mut parts = head.rsplitn(3, ':');
    let col = parts.next()?.parse().ok()?;
    let line = parts.next()?.parse().ok()?;
    let file = parts.next().filter(|f| !f.is_empty())?;
    Some((file, line, col, rest))
}

/// The two types of the last "expected X, but found Y" in `s`. A type is
/// one word or one balanced parenthesized form, so `(List (Pair a b))` is
/// read whole.
fn mismatch(s: &str) -> Option<(String, String)> {
    let i = s.rfind("expected ")?;
    let (expected, after) = balanced(&s[i + "expected ".len()..])?;
    let after = after.strip_prefix(", but found ")?;
    let (found, _) = balanced(after)?;
    Some((expected.to_string(), found.to_string()))
}

/// The leading word or parenthesized form of `s`, and what follows it.
fn balanced(s: &str) -> Option<(&str, &str)> {
    if s.starts_with('(') {
        let mut depth = 0;
        for (i, c) in s.char_indices() {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some((&s[..=i], &s[i + 1..]));
                    }
                }
                _ => {}
            }
        }
        None
    } else {
        let end = s.find([',', ' ', '.', ';', ')']).unwrap_or(s.len());
        (end > 0).then(|| (&s[..end], &s[end..]))
    }
}

/// `msg` without its design-document citations: each parenthesized aside
/// that cites `design/NN`, with the space before it. They explain the
/// language's reasons to its designers, and mean nothing to an agent
/// repairing its program; `--explain` keeps them.
pub fn strip_citations(msg: &str) -> String {
    let mut out = String::with_capacity(msg.len());
    let mut rest = msg;
    while let Some(open) = rest.find('(') {
        let Some(len) = rest[open..].find(')') else { break };
        let inner = &rest[open + 1..open + len];
        let cites = !inner.contains('(')
            && inner
                .match_indices("design/")
                .any(|(j, _)| inner[j + "design/".len()..].starts_with(|c: char| c.is_ascii_digit()));
        if cites {
            out.push_str(rest[..open].trim_end());
            rest = &rest[open + len + 1..];
        } else {
            out.push_str(&rest[..open + 1]);
            rest = &rest[open + 1..];
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn citations_go_and_the_rest_stays() {
        assert_eq!(
            strip_citations("x is already defined, at a.nrm:1:1 (a name is defined once: design/12, Q-2)"),
            "x is already defined, at a.nrm:1:1"
        );
        assert_eq!(
            strip_citations("needs f, defined below it; move the definition up (design/12 §4.4)"),
            "needs f, defined below it; move the definition up"
        );
        assert_eq!(
            strip_citations("expected (List Text), but found (Option (List Num))."),
            "expected (List Text), but found (Option (List Num))."
        );
        assert_eq!(
            strip_citations("The program was not run, because it doesn't type-check (design/12, T2)."),
            "The program was not run, because it doesn't type-check."
        );
    }

    #[test]
    fn a_type_error_reads_into_its_parts() {
        let d = Diagnostic::parse(
            Kind::Type,
            "solution.nrm:9:20: in the definition of shout: in an application: expected (List (Pair Text Num)), but found Num",
        );
        assert_eq!(d.file.as_deref(), Some("solution.nrm"));
        assert_eq!((d.line, d.column), (Some(9), Some(20)));
        assert_eq!(d.expected.as_deref(), Some("(List (Pair Text Num))"));
        assert_eq!(d.found.as_deref(), Some("Num"));
        assert_eq!(
            d.text(),
            "type error: solution.nrm:9:20: in the definition of shout: in an application: expected (List (Pair Text Num)), but found Num"
        );
    }

    #[test]
    fn labels_are_taken_off_wherever_they_are() {
        let d = Diagnostic::parse(Kind::Syntax, "syntax error: a.nrm:2:3: unexpected ')'");
        assert_eq!(d.message, "unexpected ')'");
        assert_eq!(d.text(), "syntax error: a.nrm:2:3: unexpected ')'");
        let d = Diagnostic::parse(Kind::RunTime, "a.nrm:4:1: run-time error: division by zero");
        assert_eq!(d.message, "division by zero");
        let d = Diagnostic::parse(Kind::Note, "The program was not run.");
        assert_eq!((d.file, d.message.as_str()), (None, "The program was not run."));
        // A test's message keeps what it says about the run.
        let d = Diagnostic::parse(Kind::Test, "a.nrm:5:3: run-time error: script exhausted");
        assert_eq!(d.message, "run-time error: script exhausted");
    }
}
