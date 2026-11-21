use std::collections::BTreeMap;
use std::fmt;

use crate::instructions::definition;
use crate::model::{
    Immediate, Instruction, LabelRef, Memory, Operand, Predicate, Program, Register, View,
};

pub const MAX_IMAGE_SIZE: usize = 256 * 1024 * 1024;
pub const MAX_FUNCTION_CODE_SIZE: usize = 16 * 1024 * 1024;
pub const MAX_INSTRUCTION_PAYLOAD_SIZE: usize = 16 * 1024 * 1024;
pub const MAX_INSTRUCTIONS: usize = 1_048_576;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodeError(pub String);

impl fmt::Display for EncodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for EncodeError {}

const SECTION_META: u32 = 1;
const SECTION_DATA: u32 = 3;
const SECTION_CODE: u32 = 4;
const SECTION_SYMS: u32 = 5;
const SECTION_CAPS: u32 = 8;
const SECTION_BLOCKS: u32 = 9;
const SECTION_FLAG_REQUIRED: u32 = 1;
type EncodedSection = (u32, u32, Vec<u8>);

pub fn crc32c(data: &[u8]) -> u32 {
    let mut current = 0xffff_ffff_u32;
    for byte in data {
        current ^= u32::from(*byte);
        for _ in 0..8 {
            current = (current >> 1) ^ if current & 1 != 0 { 0x82f6_3b78 } else { 0 };
        }
    }
    current ^ 0xffff_ffff
}

pub fn uleb(mut value: u64) -> Vec<u8> {
    let mut output = Vec::new();
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        output.push(byte);
        if value == 0 {
            return output;
        }
    }
}

pub fn sleb(mut value: i64) -> Vec<u8> {
    let mut output = Vec::new();
    loop {
        let mut byte = (value as u8) & 0x7f;
        value >>= 7;
        let sign = byte & 0x40 != 0;
        let more = !((value == 0 && !sign) || (value == -1 && sign));
        if more {
            byte |= 0x80;
        }
        output.push(byte);
        if !more {
            return output;
        }
    }
}

pub fn decode_uleb(data: &[u8], offset: usize) -> Result<(u64, usize), EncodeError> {
    let mut value = 0_u128;
    for index in 0..10 {
        let Some(byte) = data.get(offset.saturating_add(index)).copied() else {
            return Err(EncodeError("truncated ULEB128".into()));
        };
        value |= u128::from(byte & 0x7f) << (index * 7);
        if byte & 0x80 == 0 {
            let Some(number) = u64::try_from(value).ok() else {
                return Err(EncodeError("non-canonical or out-of-range ULEB128".into()));
            };
            let end = offset + index + 1;
            if uleb(number) != data[offset..end] {
                return Err(EncodeError("non-canonical or out-of-range ULEB128".into()));
            }
            return Ok((number, end));
        }
    }
    Err(EncodeError("ULEB128 exceeds ten bytes".into()))
}

pub fn decode_sleb(data: &[u8], offset: usize) -> Result<(i64, usize), EncodeError> {
    let mut value = 0_i128;
    for index in 0..10 {
        let Some(byte) = data.get(offset.saturating_add(index)).copied() else {
            return Err(EncodeError("truncated SLEB128".into()));
        };
        value |= i128::from(byte & 0x7f) << (index * 7);
        if byte & 0x80 == 0 {
            let bits = (index + 1) * 7;
            if byte & 0x40 != 0 {
                value -= 1_i128 << bits;
            }
            let Some(number) = i64::try_from(value).ok() else {
                return Err(EncodeError("non-canonical or out-of-range SLEB128".into()));
            };
            let end = offset + index + 1;
            if sleb(number) != data[offset..end] {
                return Err(EncodeError("non-canonical or out-of-range SLEB128".into()));
            }
            return Ok((number, end));
        }
    }
    Err(EncodeError("SLEB128 exceeds ten bytes".into()))
}

fn push_u16(output: &mut Vec<u8>, value: u16) {
    output.extend_from_slice(&value.to_le_bytes());
}
fn push_u32(output: &mut Vec<u8>, value: u32) {
    output.extend_from_slice(&value.to_le_bytes());
}
fn push_u64(output: &mut Vec<u8>, value: u64) {
    output.extend_from_slice(&value.to_le_bytes());
}

fn encode_register(register: Register, output: &mut Vec<u8>) {
    let view = match register.view {
        View::B => 0,
        View::W => 1,
        View::D => 2,
        View::Q => 3,
    };
    output.push((register.index & 0x1f) | (view << 5));
}

fn encode_operand(
    operand: &Operand,
    blocks: &BTreeMap<String, u32>,
    symbols: &BTreeMap<String, u32>,
    output: &mut Vec<u8>,
) -> Result<(), EncodeError> {
    match operand {
        Operand::Register(register) => {
            output.push(0);
            encode_register(*register, output);
        }
        Operand::Immediate(Immediate { value }) if *value >= 0 => {
            let number = u64::try_from(*value)
                .map_err(|_| EncodeError("unsigned immediate is outside u64".into()))?;
            output.push(1);
            output.extend(uleb(number));
        }
        Operand::Immediate(Immediate { value }) => {
            let number = i64::try_from(*value)
                .map_err(|_| EncodeError("signed immediate is outside i64".into()))?;
            output.push(2);
            output.extend(sleb(number));
        }
        Operand::LabelRef(LabelRef { name }) => {
            let block = blocks
                .get(name)
                .ok_or_else(|| EncodeError(format!("undefined label {:?}", name)))?;
            output.push(4);
            output.extend(uleb(u64::from(*block)));
        }
        Operand::Predicate(predicate) => {
            output.push(7);
            output.extend(uleb(predicate_code(*predicate)));
        }
        Operand::Memory(Memory { base, offset }) => {
            output.push(5);
            encode_register(*base, output);
            output.extend(sleb(i64::from(*offset)));
        }
        Operand::Symbol(name) => {
            let index = symbols
                .get(name)
                .ok_or_else(|| EncodeError(format!("undefined symbol {:?}", name)))?;
            output.push(6);
            output.extend(uleb(u64::from(*index)));
        }
    }
    Ok(())
}

fn encode_instruction(
    instruction: &Instruction,
    blocks: &BTreeMap<String, u32>,
    symbols: &BTreeMap<String, u32>,
    output: &mut Vec<u8>,
) -> Result<(), EncodeError> {
    let spec = definition(&instruction.mnemonic).ok_or_else(|| {
        EncodeError(format!(
            "unsupported v1 assembler instruction {:?}",
            instruction.mnemonic
        ))
    })?;
    let mut payload = Vec::new();
    for operand in &instruction.operands {
        encode_operand(operand, blocks, symbols, &mut payload)?;
    }
    if payload.len() > MAX_INSTRUCTION_PAYLOAD_SIZE {
        return Err(EncodeError(
            "instruction payload exceeds v1 format ceiling".into(),
        ));
    }
    let width = if spec.width {
        instruction
            .operands
            .iter()
            .enumerate()
            .find_map(|(index, operand)| match operand {
                Operand::Register(register)
                    if !matches!(instruction.mnemonic.as_str(), "extend.s" | "extend.z")
                        || index == 1 =>
                {
                    Some(register.view.code())
                }
                _ => None,
            })
            .ok_or_else(|| {
                EncodeError(format!(
                    "instruction {:?} requires a width",
                    instruction.mnemonic
                ))
            })?
    } else {
        0
    };
    push_u16(output, spec.opcode);
    output.push(
        u8::try_from(instruction.operands.len())
            .map_err(|_| EncodeError("operand count exceeds u8".into()))?,
    );
    output.push(width);
    output.extend(uleb(payload.len() as u64));
    output.extend(payload);
    Ok(())
}

fn predicate_code(predicate: Predicate) -> u64 {
    match predicate {
        Predicate::Zero => 0,
        Predicate::NotZero => 1,
        Predicate::Carry => 2,
        Predicate::NoCarry => 3,
        Predicate::Sign => 4,
        Predicate::NotSign => 5,
        Predicate::Overflow => 6,
        Predicate::NoOverflow => 7,
        Predicate::Even => 8,
        Predicate::Odd => 9,
    }
}

fn sections(program: &Program) -> Result<(Vec<EncodedSection>, bool), EncodeError> {
    for exported in &program.exports {
        if !program.labels.contains_key(exported) {
            return Err(EncodeError(format!(
                "exported symbol {:?} is not a label",
                exported
            )));
        }
    }
    if program.exports.len() > 1 {
        return Err(EncodeError(
            "initial implicit-function mode permits one exported label".into(),
        ));
    }
    if let Some(exported) = program.exports.first()
        && program.labels[exported] != 0
    {
        return Err(EncodeError(
            "implicit-function export must label instruction zero".into(),
        ));
    }
    if program.instructions.len() > MAX_INSTRUCTIONS {
        return Err(EncodeError(
            "instruction count exceeds v1 format ceiling".into(),
        ));
    }
    for instruction in &program.instructions {
        for operand in &instruction.operands {
            if let Operand::LabelRef(label) = operand
                && program
                    .labels
                    .get(&label.name)
                    .is_some_and(|position| *position >= program.instructions.len())
            {
                return Err(EncodeError(format!(
                    "branch target {:?} does not point to an instruction",
                    label.name
                )));
            }
        }
    }
    let mut positions: Vec<usize> = program.labels.values().copied().collect();
    if !program.instructions.is_empty() {
        positions.push(0);
    }
    positions.sort_unstable();
    positions.dedup();
    let blocks_by_position: BTreeMap<usize, u32> = positions
        .into_iter()
        .enumerate()
        .map(|(index, position)| (position, index as u32))
        .collect();
    let blocks: BTreeMap<String, u32> = program
        .labels
        .iter()
        .map(|(name, position)| (name.clone(), blocks_by_position[position]))
        .collect();
    let symbol_map: BTreeMap<String, u32> = program
        .exports
        .iter()
        .enumerate()
        .map(|(index, name)| (name.clone(), index as u32))
        .collect();
    let mut code = Vec::new();
    let mut instruction_offsets = Vec::with_capacity(program.instructions.len());
    for instruction in &program.instructions {
        instruction_offsets.push(code.len() as u64);
        encode_instruction(instruction, &blocks, &symbol_map, &mut code)?;
    }
    if code.len() > MAX_FUNCTION_CODE_SIZE {
        return Err(EncodeError(
            "function code exceeds v1 format ceiling".into(),
        ));
    }
    let mut code_section = Vec::new();
    push_u32(&mut code_section, 1);
    push_u32(&mut code_section, 0);
    push_u32(&mut code_section, 0);
    push_u64(&mut code_section, 36);
    push_u64(&mut code_section, code.len() as u64);
    push_u32(&mut code_section, 0);
    push_u16(&mut code_section, 0);
    push_u16(&mut code_section, 0);
    code_section.extend(code);

    let mut symbols = Vec::new();
    push_u32(&mut symbols, program.exports.len() as u32);
    for name in &program.exports {
        let encoded = name.as_bytes();
        let length = u16::try_from(encoded.len())
            .map_err(|_| EncodeError("exported symbol name exceeds u16".into()))?;
        push_u16(&mut symbols, length);
        symbols.extend_from_slice(encoded);
        push_u32(&mut symbols, 0);
    }
    let has_control_flow = program.instructions.iter().any(|instruction| {
        instruction
            .operands
            .iter()
            .any(|operand| matches!(operand, Operand::LabelRef(_)))
    });
    let mut sections = vec![
        (SECTION_META, 0, b"+A assembler 0.1\0".to_vec()),
        (SECTION_DATA, 0, Vec::new()),
        (SECTION_CODE, 0, code_section),
        (SECTION_SYMS, 0, symbols),
        (SECTION_CAPS, 0, Vec::new()),
    ];
    if has_control_flow {
        let mut block_records: Vec<(u32, u64)> = blocks_by_position
            .iter()
            .filter_map(|(instruction_index, block_id)| {
                instruction_offsets
                    .get(*instruction_index)
                    .map(|code_offset| (*block_id, *code_offset))
            })
            .collect();
        block_records.sort_unstable_by_key(|(block_id, _)| *block_id);
        let mut block_map = Vec::with_capacity(4 + block_records.len() * 16);
        push_u32(&mut block_map, block_records.len() as u32);
        for (block_id, code_offset) in block_records {
            push_u32(&mut block_map, 0);
            push_u32(&mut block_map, block_id);
            push_u64(&mut block_map, code_offset);
        }
        sections.push((SECTION_BLOCKS, SECTION_FLAG_REQUIRED, block_map));
    }
    Ok((sections, has_control_flow))
}

pub fn encode(program: &Program) -> Result<Vec<u8>, EncodeError> {
    let (sections, has_control_flow) = sections(program)?;
    let directory_offset = 40_usize;
    let mut payload_offset = directory_offset + sections.len() * 32;
    let mut directory = Vec::new();
    let mut payloads = Vec::new();
    for (section_type, flags, content) in &sections {
        let padding = (8 - payload_offset % 8) % 8;
        payloads.resize(payloads.len() + padding, 0);
        payload_offset += padding;
        push_u32(&mut directory, *section_type);
        push_u32(&mut directory, *flags);
        push_u64(&mut directory, payload_offset as u64);
        push_u64(&mut directory, content.len() as u64);
        push_u32(&mut directory, 8);
        push_u32(&mut directory, crc32c(content));
        payloads.extend_from_slice(content);
        payload_offset += content.len();
    }
    if payload_offset > MAX_IMAGE_SIZE {
        return Err(EncodeError("image exceeds v1 format ceiling".into()));
    }
    let mut image = Vec::with_capacity(payload_offset);
    image.extend_from_slice(b"A+BC");
    push_u16(&mut image, 1);
    push_u16(&mut image, u16::from(has_control_flow));
    push_u16(&mut image, 40);
    push_u16(&mut image, 0);
    push_u64(&mut image, payload_offset as u64);
    push_u64(&mut image, directory_offset as u64);
    push_u32(&mut image, sections.len() as u32);
    push_u32(
        &mut image,
        if program.exports.is_empty() {
            u32::MAX
        } else {
            0
        },
    );
    push_u32(&mut image, 0);
    image.extend(directory);
    image.extend(payloads);
    Ok(image)
}
