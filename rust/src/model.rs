use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum View {
    B,
    W,
    D,
    Q,
}

impl View {
    pub const fn bits(self) -> u32 {
        match self {
            Self::B => 8,
            Self::W => 16,
            Self::D => 32,
            Self::Q => 64,
        }
    }

    pub const fn code(self) -> u8 {
        match self {
            Self::B => 1,
            Self::W => 2,
            Self::D => 3,
            Self::Q => 4,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Register {
    pub index: u8,
    pub view: View,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Immediate {
    pub value: i128,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabelRef {
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Predicate {
    Zero,
    NotZero,
    Carry,
    NoCarry,
    Sign,
    NotSign,
    Overflow,
    NoOverflow,
    Even,
    Odd,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Memory {
    pub base: Register,
    pub offset: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Operand {
    Register(Register),
    Immediate(Immediate),
    LabelRef(LabelRef),
    Predicate(Predicate),
    Memory(Memory),
    Symbol(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instruction {
    pub mnemonic: String,
    pub operands: Vec<Operand>,
    pub line: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Program {
    pub unit: Option<String>,
    pub exports: BTreeSet<String>,
    pub labels: BTreeMap<String, usize>,
    pub instructions: Vec<Instruction>,
}
