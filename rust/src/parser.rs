use std::fmt;

use crate::instructions::{Shape, definition};
use crate::lexer::{LexError, Token, TokenKind, lex};
use crate::model::{
    Immediate, Instruction, LabelRef, Memory, Operand, Predicate, Program, Register, View,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError(pub String);

impl fmt::Display for ParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for ParseError {}

impl From<LexError> for ParseError {
    fn from(error: LexError) -> Self {
        Self(error.0)
    }
}

pub struct Parser {
    tokens: Vec<Token>,
    at: usize,
    program: Program,
}

impl Parser {
    pub fn new(source: &str) -> Result<Self, ParseError> {
        Ok(Self {
            tokens: lex(source)?,
            at: 0,
            program: Program::default(),
        })
    }

    fn current(&self) -> &Token {
        self.tokens
            .get(self.at)
            .unwrap_or_else(|| self.tokens.last().expect("lexer emits EOF"))
    }

    fn take(&mut self, kind: Option<TokenKind>) -> Result<Token, ParseError> {
        let token = self.current().clone();
        if let Some(expected) = kind
            && token.kind != expected
        {
            return Err(ParseError(format!(
                "{}:{}: expected {}, got {:?}",
                token.line,
                token.column,
                kind_name(expected),
                token.text
            )));
        }
        self.at += 1;
        Ok(token)
    }

    pub fn parse(mut self) -> Result<Program, ParseError> {
        while self.current().kind != TokenKind::Eof {
            if self.current().kind == TokenKind::Newline {
                self.take(None)?;
                continue;
            }
            self.statement()?;
            if self.current().kind == TokenKind::Newline {
                self.take(None)?;
            } else if self.current().kind != TokenKind::Eof {
                let token = self.current();
                return Err(ParseError(format!(
                    "{}:{}: expected end of line",
                    token.line, token.column
                )));
            }
        }
        Ok(self.program)
    }

    fn statement(&mut self) -> Result<(), ParseError> {
        let first = self.take(Some(TokenKind::Identifier))?;
        if self.current().kind == TokenKind::Label {
            self.take(None)?;
            if self.program.labels.contains_key(&first.text) {
                return Err(ParseError(format!(
                    "{}:{}: duplicate label {:?}",
                    first.line, first.column, first.text
                )));
            }
            self.program
                .labels
                .insert(first.text, self.program.instructions.len());
            return Ok(());
        }
        if first.text == "unit" {
            if self.program.unit.is_some() {
                return Err(ParseError(format!(
                    "{}:{}: duplicate unit declaration",
                    first.line, first.column
                )));
            }
            self.program.unit = Some(self.take(Some(TokenKind::Identifier))?.text);
            return Ok(());
        }
        if first.text == "export" {
            let name = self.take(Some(TokenKind::Identifier))?.text;
            if self.current().kind == TokenKind::Label {
                self.take(None)?;
            }
            self.program.exports.insert(name);
            return Ok(());
        }
        let spec = definition(&first.text).ok_or_else(|| {
            ParseError(format!(
                "{}:{}: unsupported v1 assembler instruction {:?}",
                first.line, first.column, first.text
            ))
        })?;
        let operands = self.instruction_operands(spec.shape, &first)?;
        self.validate(&first, spec.shape, &operands)?;
        self.program.instructions.push(Instruction {
            mnemonic: first.text,
            operands,
            line: first.line,
        });
        Ok(())
    }

    fn instruction_operands(
        &mut self,
        shape: Shape,
        first: &Token,
    ) -> Result<Vec<Operand>, ParseError> {
        match shape {
            Shape::NoArgs => Ok(Vec::new()),
            Shape::Branch | Shape::Call => {
                self.take(Some(TokenKind::Branch))?;
                Ok(vec![Operand::LabelRef(LabelRef {
                    name: self.take(Some(TokenKind::Identifier))?.text,
                })])
            }
            Shape::Two if first.text == "exchange" => {
                let first_operand = self.operand()?;
                self.take(Some(TokenKind::Swap))?;
                Ok(vec![first_operand, self.operand()?])
            }
            Shape::Two => self.comma_operands(2),
            Shape::DestinationSource | Shape::MemoryLoad | Shape::MemoryStore => {
                let destination = self.operand()?;
                self.take(Some(TokenKind::Arrow))?;
                Ok(vec![destination, self.operand()?])
            }
            Shape::DestinationTwoSourceTwo => {
                let dest1 = self.operand()?;
                self.take(Some(TokenKind::Comma))?;
                let dest2 = self.operand()?;
                self.take(Some(TokenKind::Arrow))?;
                let src1 = self.operand()?;
                self.take(Some(TokenKind::Comma))?;
                let src2 = self.operand()?;
                Ok(vec![dest1, dest2, src1, src2])
            }
            Shape::FlagSet => {
                if first.text == "flag.direction" {
                    self.take(Some(TokenKind::Arrow))?;
                    let mode_token = self.take(Some(TokenKind::Identifier))?;
                    let value = match mode_token.text.as_str() {
                        "forward" => 0,
                        "backward" => 1,
                        _ => {
                            return Err(ParseError(format!(
                                "{}:{}: expected 'forward' or 'backward', got {:?}",
                                mode_token.line, mode_token.column, mode_token.text
                            )));
                        }
                    };
                    Ok(vec![Operand::Immediate(Immediate { value })])
                } else if first.text == "raise" {
                    let code = self.operand()?;
                    Ok(vec![code])
                } else {
                    // fence acquire|release|full
                    let mode_token = self.take(Some(TokenKind::Identifier))?;
                    let value = match mode_token.text.as_str() {
                        "acquire" => 0,
                        "release" => 1,
                        "full" => 2,
                        _ => {
                            return Err(ParseError(format!(
                                "{}:{}: expected 'acquire', 'release', or 'full', got {:?}",
                                mode_token.line, mode_token.column, mode_token.text
                            )));
                        }
                    };
                    Ok(vec![Operand::Immediate(Immediate { value })])
                }
            }
            Shape::Address => {
                let destination = self.operand()?;
                self.take(Some(TokenKind::Arrow))?;
                let symbol_token = self.take(Some(TokenKind::Identifier))?;
                Ok(vec![destination, Operand::Symbol(symbol_token.text)])
            }
            Shape::CompareSwap => {
                let destination = self.operand()?;
                self.take(Some(TokenKind::Arrow))?;
                let memory = self.operand()?;
                self.take(Some(TokenKind::Comma))?;
                let expected = self.operand()?;
                self.take(Some(TokenKind::Comma))?;
                let desired = self.operand()?;
                Ok(vec![destination, memory, expected, desired])
            }
            Shape::Sys => Ok(Vec::new()),
            Shape::DestinationTwo => {
                let destination = self.operand()?;
                self.take(Some(TokenKind::Arrow))?;
                let mut operands = vec![destination];
                operands.extend(self.comma_operands(2)?);
                Ok(operands)
            }
            Shape::Select => {
                let destination = self.operand()?;
                self.take(Some(TokenKind::Arrow))?;
                let predicate_token = self.take(Some(TokenKind::Identifier))?;
                let predicate = parse_predicate(&predicate_token)?;
                self.take(Some(TokenKind::Comma))?;
                let when_true = self.operand()?;
                self.take(Some(TokenKind::Comma))?;
                let when_false = self.operand()?;
                Ok(vec![
                    destination,
                    Operand::Predicate(predicate),
                    when_true,
                    when_false,
                ])
            }
        }
    }

    fn comma_operands(&mut self, count: usize) -> Result<Vec<Operand>, ParseError> {
        let mut values = vec![self.operand()?];
        while values.len() < count {
            self.take(Some(TokenKind::Comma))?;
            values.push(self.operand()?);
        }
        Ok(values)
    }

    fn operand(&mut self) -> Result<Operand, ParseError> {
        let token = self.current().clone();
        match token.kind {
            TokenKind::Number => {
                self.take(None)?;
                let value = parse_integer(&token.text).ok_or_else(|| invalid_number(&token))?;
                Ok(Operand::Immediate(Immediate { value }))
            }
            TokenKind::Identifier => {
                self.take(None)?;
                Ok(Operand::Register(self.register(&token)?))
            }
            TokenKind::LeftBracket => {
                self.take(None)?;
                let base_token = self.take(Some(TokenKind::Identifier))?;
                let base = self.register(&base_token)?;
                let offset = if self.current().kind == TokenKind::Plus {
                    self.take(None)?;
                    let number = self.take(Some(TokenKind::Number))?;
                    parse_integer(&number.text).ok_or_else(|| invalid_number(&number))?
                } else if self.current().kind == TokenKind::Number
                    && self.current().text.starts_with('-')
                {
                    let number = self.take(None)?;
                    parse_integer(&number.text).ok_or_else(|| invalid_number(&number))?
                } else {
                    0
                };
                if offset < i32::MIN as i128 || offset > i32::MAX as i128 {
                    return Err(ParseError(format!(
                        "{}:{}: memory offset does not fit 32 bits",
                        token.line, token.column
                    )));
                }
                self.take(Some(TokenKind::RightBracket))?;
                Ok(Operand::Memory(Memory {
                    base,
                    offset: offset as i32,
                }))
            }
            _ => Err(ParseError(format!(
                "{}:{}: expected operand",
                token.line, token.column
            ))),
        }
    }

    fn register(&self, token: &Token) -> Result<Register, ParseError> {
        let parts: Vec<&str> = token.text.split('.').collect();
        let root = parts[0];
        let Some(index_text) = root.strip_prefix('R') else {
            return Err(ParseError(format!(
                "{}:{}: expected R0 through R31, got {:?}",
                token.line, token.column, token.text
            )));
        };
        if index_text.is_empty()
            || !index_text
                .chars()
                .all(|character| character.is_ascii_digit())
        {
            return Err(ParseError(format!(
                "{}:{}: expected R0 through R31, got {:?}",
                token.line, token.column, token.text
            )));
        }
        let index = index_text.parse::<u8>().ok();
        if index.is_none_or(|value| value.to_string() != index_text) {
            return Err(ParseError(format!(
                "{}:{}: expected R0 through R31, got {:?}",
                token.line, token.column, token.text
            )));
        }
        let index = index.expect("checked above");
        if index > 31 {
            return Err(ParseError(format!(
                "{}:{}: register index out of range",
                token.line, token.column
            )));
        }
        let view = match parts.as_slice() {
            [_] => View::Q,
            [_, "b"] => View::B,
            [_, "w"] => View::W,
            [_, "d"] => View::D,
            [_, "q"] => View::Q,
            _ => {
                return Err(ParseError(format!(
                    "{}:{}: invalid register view {:?}",
                    token.line, token.column, token.text
                )));
            }
        };
        Ok(Register { index, view })
    }

    fn validate(
        &self,
        first: &Token,
        shape: Shape,
        operands: &[Operand],
    ) -> Result<(), ParseError> {
        let spec = definition(&first.text).expect("validated instruction definition");
        let registers: Vec<Register> = operands
            .iter()
            .filter_map(|operand| match operand {
                Operand::Register(register) => Some(*register),
                _ => None,
            })
            .collect();
        if spec.width && registers.is_empty() {
            return Err(ParseError(format!(
                "{}:{}: instruction {:?} requires at least one register to determine width",
                first.line, first.column, first.text
            )));
        }
        if matches!(
            shape,
            Shape::DestinationSource
                | Shape::DestinationTwo
                | Shape::DestinationTwoSourceTwo
                | Shape::MemoryLoad
                | Shape::Select
                | Shape::Address
        ) && !matches!(operands.first(), Some(Operand::Register(_)))
        {
            return Err(ParseError(format!(
                "{}:{}: destination must be a register",
                first.line, first.column
            )));
        }
        if shape == Shape::MemoryLoad && !matches!(operands.get(1), Some(Operand::Memory(_))) {
            return Err(ParseError(format!(
                "{}:{}: load source must be memory",
                first.line, first.column
            )));
        }
        if shape == Shape::MemoryStore && !matches!(operands.first(), Some(Operand::Memory(_))) {
            return Err(ParseError(format!(
                "{}:{}: store destination must be memory",
                first.line, first.column
            )));
        }
        if shape == Shape::CompareSwap {
            if !matches!(operands.first(), Some(Operand::Register(_))) {
                return Err(ParseError(format!(
                    "{}:{}: compare.swap destination must be a register",
                    first.line, first.column
                )));
            }
            if !matches!(operands.get(1), Some(Operand::Memory(_))) {
                return Err(ParseError(format!(
                    "{}:{}: compare.swap expects memory operand",
                    first.line, first.column
                )));
            }
        }
        for operand in operands {
            if let Operand::Memory(memory) = operand
                && memory.base.view != View::Q
            {
                return Err(ParseError(format!(
                    "{}:{}: memory base must be a full GPR",
                    first.line, first.column
                )));
            }
        }
        if first.text == "exchange" {
            if !operands
                .iter()
                .all(|operand| matches!(operand, Operand::Register(Register { view: View::Q, .. })))
            {
                return Err(ParseError(format!(
                    "{}:{}: exchange requires full GPRs",
                    first.line, first.column
                )));
            }
            if operands.first() == operands.get(1) {
                return Err(ParseError(format!(
                    "{}:{}: exchange operands must differ",
                    first.line, first.column
                )));
            }
        }
        if matches!(first.text.as_str(), "extend.s" | "extend.z") {
            let (Some(Operand::Register(destination)), Some(Operand::Register(source))) =
                (operands.first(), operands.get(1))
            else {
                return Err(ParseError(format!(
                    "{}:{}: extension operands must be registers",
                    first.line, first.column
                )));
            };
            if source.view.bits() >= destination.view.bits() {
                return Err(ParseError(format!(
                    "{}:{}: extension source must be narrower than destination",
                    first.line, first.column
                )));
            }
        }
        if first.text != "exchange"
            && !matches!(first.text.as_str(), "extend.s" | "extend.z")
            && registers
                .iter()
                .any(|register| register.view != registers[0].view)
        {
            return Err(ParseError(format!(
                "{}:{}: register operand widths must match",
                first.line, first.column
            )));
        }
        let memory_width = match first.text.as_str() {
            "load.u8" | "store.u8" | "load.acquire.u8" | "store.release.u8" => Some(View::B),
            "load.u16" | "store.u16" | "load.acquire.u16" | "store.release.u16" => Some(View::W),
            "load.u32" | "store.u32" | "load.acquire.u32" | "store.release.u32" => Some(View::D),
            "load.u64" | "store.u64" | "load.acquire.u64" | "store.release.u64" => Some(View::Q),
            _ => None,
        };
        if let Some(expected) = memory_width {
            let register_operand = if shape == Shape::MemoryLoad {
                operands.first()
            } else {
                operands.get(1)
            };
            if !matches!(register_operand, Some(Operand::Register(register)) if register.view == expected)
            {
                return Err(ParseError(format!(
                    "{}:{}: {} requires a {}-bit register view",
                    first.line,
                    first.column,
                    first.text,
                    expected.bits()
                )));
            }
        }
        if let Some(register) = registers.first() {
            let operation_view = if matches!(first.text.as_str(), "extend.s" | "extend.z") {
                registers[1].view
            } else {
                register.view
            };
            let minimum = -(1_i128 << (operation_view.bits() - 1));
            let maximum = (1_i128 << operation_view.bits()) - 1;
            for operand in operands {
                if let Operand::Immediate(immediate) = operand
                    && (immediate.value < minimum || immediate.value > maximum)
                {
                    return Err(ParseError(format!(
                        "{}:{}: immediate does not fit {} bits",
                        first.line,
                        first.column,
                        operation_view.bits()
                    )));
                }
            }
        }
        Ok(())
    }
}

fn parse_predicate(token: &Token) -> Result<Predicate, ParseError> {
    match token.text.as_str() {
        "zero" => Ok(Predicate::Zero),
        "notzero" => Ok(Predicate::NotZero),
        "carry" => Ok(Predicate::Carry),
        "nocarry" => Ok(Predicate::NoCarry),
        "sign" => Ok(Predicate::Sign),
        "notsign" => Ok(Predicate::NotSign),
        "overflow" => Ok(Predicate::Overflow),
        "nooverflow" => Ok(Predicate::NoOverflow),
        "even" => Ok(Predicate::Even),
        "odd" => Ok(Predicate::Odd),
        _ => Err(ParseError(format!(
            "{}:{}: unknown flag predicate {:?}",
            token.line, token.column, token.text
        ))),
    }
}

fn kind_name(kind: TokenKind) -> &'static str {
    match kind {
        TokenKind::Identifier => "IDENT",
        TokenKind::Number => "NUMBER",
        TokenKind::Label => "LABEL",
        TokenKind::Arrow => "ARROW",
        TokenKind::Branch => "BRANCH",
        TokenKind::Swap => "SWAP",
        TokenKind::LeftBracket => "LBRACK",
        TokenKind::RightBracket => "RBRACK",
        TokenKind::Comma => "COMMA",
        TokenKind::Plus => "PLUS",
        TokenKind::Newline => "NEWLINE",
        TokenKind::Eof => "EOF",
    }
}

fn invalid_number(token: &Token) -> ParseError {
    ParseError(format!(
        "{}:{}: invalid number literal {:?}",
        token.line, token.column, token.text
    ))
}

fn parse_integer(text: &str) -> Option<i128> {
    let (negative, unsigned) = text
        .strip_prefix('-')
        .map_or((false, text), |value| (true, value));
    let (radix, digits) = if let Some(value) = unsigned
        .strip_prefix("0x")
        .or_else(|| unsigned.strip_prefix("0X"))
    {
        (16, value)
    } else if let Some(value) = unsigned
        .strip_prefix("0b")
        .or_else(|| unsigned.strip_prefix("0B"))
    {
        (2, value)
    } else {
        (10, unsigned)
    };
    if digits.is_empty() || digits.ends_with('_') || digits.contains("__") {
        return None;
    }
    let normalized = digits.replace('_', "");
    if normalized.is_empty() || digits.starts_with('_') && digits.len() > normalized.len() + 1 {
        return None;
    }
    if radix == 10
        && normalized.len() > 1
        && normalized.starts_with('0')
        && normalized.chars().any(|c| c != '0')
    {
        return None;
    }
    if !normalized
        .chars()
        .all(|character| character.is_digit(radix))
    {
        return None;
    }
    let magnitude = u128::from_str_radix(&normalized, radix).unwrap_or(u128::MAX);
    if negative {
        if magnitude >= (1_u128 << 127) {
            Some(i128::MIN)
        } else {
            Some(-(magnitude as i128))
        }
    } else if magnitude > i128::MAX as u128 {
        Some(i128::MAX)
    } else {
        Some(magnitude as i128)
    }
}

pub fn parse(source: &str) -> Result<Program, ParseError> {
    Parser::new(source)?.parse()
}
