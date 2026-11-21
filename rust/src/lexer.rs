use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    Identifier,
    Number,
    Label,
    Arrow,
    Branch,
    Swap,
    LeftBracket,
    RightBracket,
    Comma,
    Plus,
    Newline,
    Eof,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    pub text: String,
    pub line: usize,
    pub column: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LexError(pub String);

impl fmt::Display for LexError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for LexError {}

fn is_ident_start(character: char) -> bool {
    character.is_ascii_alphabetic() || character == '_'
}

fn is_ident_continue(character: char) -> bool {
    character.is_ascii_alphanumeric() || matches!(character, '_' | '.' | '!')
}

pub fn lex(source: &str) -> Result<Vec<Token>, LexError> {
    let characters: Vec<char> = source.chars().collect();
    let mut tokens = Vec::new();
    let (mut position, mut line, mut column) = (0, 1, 1);
    while position < characters.len() {
        let current = characters[position];
        if matches!(current, ' ' | '\t') {
            position += 1;
            column += 1;
            continue;
        }
        if current == ';' && characters.get(position + 1) == Some(&';') {
            while position < characters.len() && characters[position] != '\n' {
                position += 1;
                column += 1;
            }
            continue;
        }
        if current == '\n' {
            tokens.push(Token {
                kind: TokenKind::Newline,
                text: "\n".into(),
                line,
                column,
            });
            position += 1;
            line += 1;
            column = 1;
            continue;
        }

        let start = position;
        let kind = if characters.get(position..position + 3) == Some(&['<', '-', '>']) {
            position += 3;
            TokenKind::Swap
        } else if characters.get(position..position + 2) == Some(&['<', '-']) {
            position += 2;
            TokenKind::Arrow
        } else if characters.get(position..position + 2) == Some(&['-', '>']) {
            position += 2;
            TokenKind::Branch
        } else if characters.get(position..position + 2) == Some(&[':', ':']) {
            position += 2;
            TokenKind::Label
        } else {
            match current {
                '[' => {
                    position += 1;
                    TokenKind::LeftBracket
                }
                ']' => {
                    position += 1;
                    TokenKind::RightBracket
                }
                ',' => {
                    position += 1;
                    TokenKind::Comma
                }
                '+' => {
                    position += 1;
                    TokenKind::Plus
                }
                '-' if characters
                    .get(position + 1)
                    .is_some_and(|c| c.is_ascii_digit()) =>
                {
                    position = scan_number(&characters, position);
                    TokenKind::Number
                }
                character if character.is_ascii_digit() => {
                    position = scan_number(&characters, position);
                    TokenKind::Number
                }
                character if is_ident_start(character) => {
                    position += 1;
                    while characters
                        .get(position)
                        .is_some_and(|c| is_ident_continue(*c))
                    {
                        position += 1;
                    }
                    TokenKind::Identifier
                }
                _ => {
                    return Err(LexError(format!(
                        "{line}:{column}: unexpected character {current:?}"
                    )));
                }
            }
        };
        let text: String = characters[start..position].iter().collect();
        tokens.push(Token {
            kind,
            text,
            line,
            column,
        });
        column += position - start;
    }
    tokens.push(Token {
        kind: TokenKind::Eof,
        text: String::new(),
        line,
        column,
    });
    Ok(tokens)
}

fn scan_number(characters: &[char], mut position: usize) -> usize {
    if characters[position] == '-' {
        position += 1;
    }
    let prefix = characters.get(position..position + 2);
    let radix = match prefix {
        Some(['0', 'x' | 'X']) => Some(16),
        Some(['0', 'b' | 'B']) => Some(2),
        _ => None,
    };
    let valid_prefix_digit =
        |character: char, radix: u32| character == '_' || character.is_digit(radix);
    if let Some(radix) = radix.filter(|radix| {
        characters
            .get(position + 2)
            .is_some_and(|character| valid_prefix_digit(*character, *radix))
    }) {
        position += 2;
        while characters
            .get(position)
            .is_some_and(|character| valid_prefix_digit(*character, radix))
        {
            position += 1;
        }
    } else {
        while characters
            .get(position)
            .is_some_and(|c| c.is_ascii_digit() || *c == '_')
        {
            position += 1;
        }
    }
    position
}
