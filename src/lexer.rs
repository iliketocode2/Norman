//! Lexical structure (04 §1): comments, brackets, strings, atoms, and the
//! reader that turns them into S-expressions.

use crate::ast::Loc;
use std::fmt;
use std::rc::Rc;

/// An S-expression: the tree the parser consumes.
#[derive(Debug, Clone)]
pub enum Sx {
    Atom(Rc<str>, Loc),
    Str(Rc<str>, Loc),
    List(Vec<Sx>, Loc),
}

impl Sx {
    pub fn loc(&self) -> &Loc {
        match self {
            Sx::Atom(_, l) | Sx::Str(_, l) | Sx::List(_, l) => l,
        }
    }
    pub fn atom(&self) -> Option<&str> {
        match self {
            Sx::Atom(a, _) => Some(a),
            _ => None,
        }
    }
    pub fn list(&self) -> Option<&[Sx]> {
        match self {
            Sx::List(xs, _) => Some(xs),
            _ => None,
        }
    }
}

impl fmt::Display for Sx {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Sx::Atom(a, _) => write!(f, "{}", a),
            Sx::Str(s, _) => write!(f, "{}", quote(s)),
            Sx::List(xs, _) => {
                write!(f, "(")?;
                for (i, x) in xs.iter().enumerate() {
                    if i > 0 {
                        write!(f, " ")?;
                    }
                    write!(f, "{}", x)?;
                }
                write!(f, ")")
            }
        }
    }
}

/// Render a string as a µNorman string literal.
pub fn quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Read every S-expression in `src`.
pub fn read_all(src: &str, file: &str) -> Result<Vec<Sx>, String> {
    let mut r = Reader {
        chars: src.chars().collect(),
        pos: 0,
        line: 1,
        col: 1,
        file: Rc::from(file),
    };
    let mut out = vec![];
    loop {
        r.skip_ws();
        if r.pos >= r.chars.len() {
            return Ok(out);
        }
        out.push(r.read()?);
    }
}

struct Reader {
    chars: Vec<char>,
    pos: usize,
    line: u32,
    col: u32,
    file: Rc<str>,
}

impl Reader {
    fn loc(&self) -> Loc {
        Loc { file: self.file.clone(), line: self.line, col: self.col }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += 1;
        if c == '\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        Some(c)
    }

    /// Whitespace and `;` comments separate tokens and are otherwise ignored.
    fn skip_ws(&mut self) {
        while let Some(c) = self.peek() {
            if c == ';' {
                while let Some(c) = self.peek() {
                    if c == '\n' {
                        break;
                    }
                    self.bump();
                }
            } else if c.is_whitespace() {
                self.bump();
            } else {
                break;
            }
        }
    }

    fn read(&mut self) -> Result<Sx, String> {
        self.skip_ws();
        let loc = self.loc();
        match self.peek() {
            None => Err(format!("{}: unexpected end of input", loc)),
            // Round and square brackets are interchangeable, provided they match.
            Some(open @ ('(' | '[')) => {
                self.bump();
                let close = if open == '(' { ')' } else { ']' };
                let mut items = vec![];
                loop {
                    self.skip_ws();
                    match self.peek() {
                        None => return Err(format!("{}: unclosed '{}'", loc, open)),
                        Some(c) if c == close => {
                            self.bump();
                            return Ok(Sx::List(items, loc));
                        }
                        Some(c @ (')' | ']')) => {
                            return Err(format!("{}: '{}' does not match '{}' opened at {}", self.loc(), c, open, loc))
                        }
                        _ => items.push(self.read()?),
                    }
                }
            }
            Some(c @ (')' | ']')) => Err(format!("{}: unexpected '{}'", loc, c)),
            Some('"') => {
                self.bump();
                let mut s = String::new();
                loop {
                    match self.bump() {
                        None => return Err(format!("{}: unterminated string", loc)),
                        Some('"') => return Ok(Sx::Str(Rc::from(s.as_str()), loc)),
                        Some('\\') => match self.bump() {
                            Some('n') => s.push('\n'),
                            Some('t') => s.push('\t'),
                            Some('"') => s.push('"'),
                            Some('\\') => s.push('\\'),
                            other => return Err(format!("{}: unknown escape \\{}", loc, other.unwrap_or(' '))),
                        },
                        Some(c) => s.push(c),
                    }
                }
            }
            Some('\'') => {
                self.bump();
                if self.peek() == Some('(') {
                    self.bump();
                    self.skip_ws();
                    if self.peek() == Some(')') {
                        self.bump();
                        return Ok(Sx::Atom(Rc::from("'()"), loc));
                    }
                    return Err(format!("{}: the only quoted list is '()", loc));
                }
                let a = self.atom_chars();
                if a.is_empty() {
                    return Err(format!("{}: nothing follows the quote", loc));
                }
                Ok(Sx::Atom(Rc::from(format!("'{}", a).as_str()), loc))
            }
            Some(_) => {
                let a = self.atom_chars();
                Ok(Sx::Atom(Rc::from(a.as_str()), loc))
            }
        }
    }

    /// Other characters clump into tokens that are as long as possible.
    fn atom_chars(&mut self) -> String {
        let mut s = String::new();
        while let Some(c) = self.peek() {
            if c.is_whitespace() || "()[];\"".contains(c) {
                break;
            }
            s.push(c);
            self.bump();
        }
        s
    }
}
