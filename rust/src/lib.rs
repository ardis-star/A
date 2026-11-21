//! +A v1 source assembler and canonical `.avm` encoder.

pub mod avm;
pub mod bytecode;
pub mod cli;
pub mod instructions;
pub mod jit;
pub mod lexer;
pub mod model;
pub mod native;
pub mod parser;
pub mod verifier;

use std::fmt;

use model::Program;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssemblyError(pub String);

impl fmt::Display for AssemblyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for AssemblyError {}

pub fn assemble(source: &str) -> Result<Vec<u8>, AssemblyError> {
    let program = parser::parse(source).map_err(|error| AssemblyError(error.0))?;
    bytecode::encode(&program).map_err(|error| AssemblyError(error.0))
}

pub fn assemble_program(program: &Program) -> Result<Vec<u8>, AssemblyError> {
    bytecode::encode(program).map_err(|error| AssemblyError(error.0))
}
