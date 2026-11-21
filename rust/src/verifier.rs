use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use crate::bytecode::{
    MAX_FUNCTION_CODE_SIZE, MAX_IMAGE_SIZE, MAX_INSTRUCTION_PAYLOAD_SIZE, MAX_INSTRUCTIONS, crc32c,
    decode_sleb, decode_uleb,
};
use crate::instructions::{Shape, definition_by_opcode};
use crate::model::{Predicate, View};

const HEADER_SIZE: usize = 40;
const SECTION_ENTRY_SIZE: usize = 32;
const MAX_SECTIONS: usize = 256;
const MAX_FUNCTIONS: usize = 65_536;
const SECTION_DATA: u32 = 3;
const SECTION_CODE: u32 = 4;
const SECTION_SYMS: u32 = 5;
const SECTION_CAPS: u32 = 8;
const SECTION_EXCP: u32 = 7;
const SECTION_BLOCKS: u32 = 9;
const SECTION_FLAG_REQUIRED: u32 = 1;
const CAP_MEMORY_MAP: u32 = 1 << 0;
const CAP_IO_PORT: u32 = 1 << 1;
const CAP_IRQ_MANAGE: u32 = 1 << 2;
const CAP_CONTEXT_SWITCH: u32 = 1 << 3;
const CAP_CPU_CONTROL: u32 = 1 << 4;
const CAP_POWER_CONTROL: u32 = 1 << 5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyError(pub String);

impl fmt::Display for VerifyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for VerifyError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Section<'a> {
    pub kind: u32,
    pub flags: u32,
    pub offset: usize,
    pub data: &'a [u8],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerifiedRegister {
    pub index: u8,
    pub view: View,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifiedOperand {
    Register(VerifiedRegister),
    Unsigned(u64),
    Signed(i64),
    Block(u64),
    Memory { base: VerifiedRegister, offset: i64 },
    Predicate(Predicate),
    Symbol(u32),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedInstruction {
    pub opcode: u16,
    pub width: Option<View>,
    pub code_offset: usize,
    pub operands: Vec<VerifiedOperand>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedFunction {
    pub id: u32,
    pub flags: u32,
    pub code_offset: usize,
    pub code_length: usize,
    pub stack_frame_maximum: u32,
    pub argument_count: u16,
    pub result_count: u16,
    pub instructions: Vec<VerifiedInstruction>,
    pub blocks: BTreeMap<u64, usize>,
}

/// One entry of the SYMS section: an exported name bound to a function slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    pub name: String,
    pub function: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedImage {
    pub minor_version: u16,
    pub entry_function: Option<u32>,
    pub functions: Vec<VerifiedFunction>,
    pub symbols: Vec<Symbol>,
    pub sections: BTreeMap<u32, Vec<u8>>,
    pub unresolved_control_targets: Vec<u64>,
}

fn fail(message: impl Into<String>) -> VerifyError {
    VerifyError(message.into())
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, VerifyError> {
    let data = bytes
        .get(
            offset
                ..offset
                    .checked_add(2)
                    .ok_or_else(|| fail("offset overflow"))?,
        )
        .ok_or_else(|| fail("truncated u16 field"))?;
    Ok(u16::from_le_bytes(
        data.try_into().expect("slice size checked"),
    ))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, VerifyError> {
    let data = bytes
        .get(
            offset
                ..offset
                    .checked_add(4)
                    .ok_or_else(|| fail("offset overflow"))?,
        )
        .ok_or_else(|| fail("truncated u32 field"))?;
    Ok(u32::from_le_bytes(
        data.try_into().expect("slice size checked"),
    ))
}

fn read_u64(bytes: &[u8], offset: usize) -> Result<u64, VerifyError> {
    let data = bytes
        .get(
            offset
                ..offset
                    .checked_add(8)
                    .ok_or_else(|| fail("offset overflow"))?,
        )
        .ok_or_else(|| fail("truncated u64 field"))?;
    Ok(u64::from_le_bytes(
        data.try_into().expect("slice size checked"),
    ))
}

fn bounded_range(
    offset: u64,
    length: u64,
    bound: usize,
    what: &str,
) -> Result<std::ops::Range<usize>, VerifyError> {
    let end = offset
        .checked_add(length)
        .ok_or_else(|| fail(format!("{what} range overflow")))?;
    let start =
        usize::try_from(offset).map_err(|_| fail(format!("{what} offset is not representable")))?;
    let end = usize::try_from(end).map_err(|_| fail(format!("{what} end is not representable")))?;
    if end > bound || start > end {
        return Err(fail(format!("{what} is outside its containing range")));
    }
    Ok(start..end)
}

fn decode_register(byte: u8) -> Result<VerifiedRegister, VerifyError> {
    let view = match byte >> 5 {
        0 => View::B,
        1 => View::W,
        2 => View::D,
        3 => View::Q,
        _ => return Err(fail("register operand has reserved view bits")),
    };
    Ok(VerifiedRegister {
        index: byte & 0x1f,
        view,
    })
}

fn decode_instruction_stream(
    code: &[u8],
    base: usize,
) -> Result<(Vec<VerifiedInstruction>, Vec<u64>), VerifyError> {
    let mut instructions = Vec::new();
    let mut control_targets = Vec::new();
    let mut position = 0usize;
    while position < code.len() {
        if instructions.len() >= MAX_INSTRUCTIONS {
            return Err(fail("instruction count exceeds v1 format ceiling"));
        }
        let header_end = position
            .checked_add(4)
            .ok_or_else(|| fail("instruction header overflow"))?;
        let header = code
            .get(position..header_end)
            .ok_or_else(|| fail("truncated instruction header"))?;
        let opcode = u16::from_le_bytes([header[0], header[1]]);
        let operand_count = usize::from(header[2]);
        let width_code = header[3];
        let definition = definition_by_opcode(opcode)
            .ok_or_else(|| fail(format!("unknown opcode 0x{opcode:04x}")))?;
        let width = match width_code {
            0 => None,
            1 => Some(View::B),
            2 => Some(View::W),
            3 => Some(View::D),
            4 => Some(View::Q),
            _ => return Err(fail("instruction has invalid width code")),
        };
        if definition.width != width.is_some() {
            return Err(fail("opcode and instruction width code disagree"));
        }
        let (payload_length, payload_start) =
            decode_uleb(code, position + 4).map_err(|error| fail(error.to_string()))?;
        if payload_length > MAX_INSTRUCTION_PAYLOAD_SIZE as u64 {
            return Err(fail("instruction payload exceeds v1 format ceiling"));
        }
        let payload_range = bounded_range(
            payload_start as u64,
            payload_length,
            code.len(),
            "instruction payload",
        )?;
        let payload = &code[payload_range.clone()];
        let mut cursor = 0usize;
        let mut operands = Vec::with_capacity(operand_count);
        for _ in 0..operand_count {
            let tag = *payload
                .get(cursor)
                .ok_or_else(|| fail("operand count exceeds payload"))?;
            cursor += 1;
            let operand = match tag {
                0 => {
                    let byte = *payload
                        .get(cursor)
                        .ok_or_else(|| fail("truncated register operand"))?;
                    cursor += 1;
                    VerifiedOperand::Register(decode_register(byte)?)
                }
                1 => {
                    let (value, end) =
                        decode_uleb(payload, cursor).map_err(|error| fail(error.to_string()))?;
                    cursor = end;
                    VerifiedOperand::Unsigned(value)
                }
                2 => {
                    let (value, end) =
                        decode_sleb(payload, cursor).map_err(|error| fail(error.to_string()))?;
                    cursor = end;
                    VerifiedOperand::Signed(value)
                }
                4 => {
                    let (value, end) =
                        decode_uleb(payload, cursor).map_err(|error| fail(error.to_string()))?;
                    cursor = end;
                    control_targets.push(value);
                    VerifiedOperand::Block(value)
                }
                5 => {
                    let byte = *payload
                        .get(cursor)
                        .ok_or_else(|| fail("truncated memory base register"))?;
                    cursor += 1;
                    let base = decode_register(byte)?;
                    if base.view != View::Q {
                        return Err(fail("memory base must be a full GPR"));
                    }
                    let (offset, end) =
                        decode_sleb(payload, cursor).map_err(|error| fail(error.to_string()))?;
                    cursor = end;
                    if i32::try_from(offset).is_err() {
                        return Err(fail("memory offset does not fit 32 bits"));
                    }
                    VerifiedOperand::Memory { base, offset }
                }
                7 => {
                    let (value, end) =
                        decode_uleb(payload, cursor).map_err(|error| fail(error.to_string()))?;
                    cursor = end;
                    VerifiedOperand::Predicate(match value {
                        0 => Predicate::Zero,
                        1 => Predicate::NotZero,
                        2 => Predicate::Carry,
                        3 => Predicate::NoCarry,
                        4 => Predicate::Sign,
                        5 => Predicate::NotSign,
                        6 => Predicate::Overflow,
                        7 => Predicate::NoOverflow,
                        8 => Predicate::Even,
                        9 => Predicate::Odd,
                        _ => return Err(fail("unknown predicate operand")),
                    })
                }
                3 => {
                    return Err(fail(format!(
                        "operand tag {tag} is not supported by this v1 assembler"
                    )));
                }
                6 => {
                    let (value, end) =
                        decode_uleb(payload, cursor).map_err(|error| fail(error.to_string()))?;
                    cursor = end;
                    VerifiedOperand::Symbol(
                        u32::try_from(value).map_err(|_| fail("symbol index is outside u32"))?,
                    )
                }
                _ => return Err(fail(format!("unknown operand tag {tag}"))),
            };
            operands.push(operand);
        }
        if cursor != payload.len() {
            return Err(fail("operand payload was not consumed exactly"));
        }
        validate_instruction(opcode, definition.shape, width, &operands)?;
        let register_views: Vec<View> = operands
            .iter()
            .filter_map(|operand| match operand {
                VerifiedOperand::Register(register) => Some(register.view),
                _ => None,
            })
            .collect();
        if let Some(operation_width) = width {
            if !matches!(opcode, 0x0011 | 0x0012)
                && register_views.iter().any(|view| *view != operation_width)
            {
                return Err(fail(
                    "register operand widths do not match instruction width",
                ));
            }
            for operand in &operands {
                let value = match operand {
                    VerifiedOperand::Unsigned(value) => Some(i128::from(*value)),
                    VerifiedOperand::Signed(value) => Some(i128::from(*value)),
                    _ => None,
                };
                if let Some(value) = value {
                    let minimum = -(1_i128 << (operation_width.bits() - 1));
                    let maximum = (1_i128 << operation_width.bits()) - 1;
                    if value < minimum || value > maximum {
                        return Err(fail("immediate does not fit instruction width"));
                    }
                }
            }
        }
        let next = payload_range.end;
        instructions.push(VerifiedInstruction {
            opcode,
            width,
            code_offset: base + position,
            operands,
        });
        position = next;
    }
    if position != code.len() {
        return Err(fail("instruction stream does not end on a record boundary"));
    }
    Ok((instructions, control_targets))
}

fn validate_instruction(
    opcode: u16,
    shape: Shape,
    width: Option<View>,
    operands: &[VerifiedOperand],
) -> Result<(), VerifyError> {
    let register = |index: usize| matches!(operands.get(index), Some(VerifiedOperand::Register(_)));
    let scalar = |index: usize| {
        matches!(
            operands.get(index),
            Some(
                VerifiedOperand::Register(_)
                    | VerifiedOperand::Unsigned(_)
                    | VerifiedOperand::Signed(_)
            )
        )
    };
    let exact_count = match shape {
        Shape::NoArgs => 0,
        Shape::Branch | Shape::Call => 1,
        Shape::Two => 2,
        Shape::DestinationSource | Shape::MemoryLoad | Shape::MemoryStore | Shape::Address => 2,
        Shape::DestinationTwo => 3,
        Shape::DestinationTwoSourceTwo => 4,
        Shape::FlagSet => 1,
        Shape::CompareSwap => 4,
        Shape::Select => 4,
        Shape::Sys => 0,
    };
    if operands.len() != exact_count {
        return Err(fail("opcode has invalid operand count"));
    }
    let valid = match shape {
        Shape::NoArgs => true,
        Shape::Branch | Shape::Call => matches!(operands.first(), Some(VerifiedOperand::Block(_))),
        Shape::Two => operands.iter().all(|operand| {
            matches!(
                operand,
                VerifiedOperand::Register(_)
                    | VerifiedOperand::Unsigned(_)
                    | VerifiedOperand::Signed(_)
            )
        }),
        Shape::DestinationSource => register(0) && scalar(1),
        Shape::Address => {
            register(0) && matches!(operands.get(1), Some(VerifiedOperand::Symbol(_)))
        }
        Shape::DestinationTwo => register(0) && scalar(1) && scalar(2),
        Shape::DestinationTwoSourceTwo => register(0) && register(1) && scalar(2) && scalar(3),
        Shape::FlagSet => matches!(
            operands.first(),
            Some(VerifiedOperand::Unsigned(_)) | Some(VerifiedOperand::Signed(_))
        ),
        Shape::CompareSwap => {
            register(0)
                && matches!(operands.get(1), Some(VerifiedOperand::Memory { .. }))
                && scalar(2)
                && scalar(3)
        }
        Shape::Sys => true,
        Shape::Select => {
            register(0)
                && matches!(operands.get(1), Some(VerifiedOperand::Predicate(_)))
                && scalar(2)
                && scalar(3)
        }
        Shape::MemoryLoad => {
            register(0) && matches!(operands.get(1), Some(VerifiedOperand::Memory { .. }))
        }
        Shape::MemoryStore => {
            matches!(operands.first(), Some(VerifiedOperand::Memory { .. })) && scalar(1)
        }
    };
    if !valid {
        return Err(fail("opcode has invalid operand forms"));
    }
    if opcode == 0x0060
        && operands.iter().any(|operand| {
            !matches!(
                operand,
                VerifiedOperand::Register(VerifiedRegister { view: View::Q, .. })
            )
        })
    {
        return Err(fail("exchange requires full GPRs"));
    }
    if matches!(opcode, 0x0011 | 0x0012) {
        let (Some(VerifiedOperand::Register(destination)), Some(VerifiedOperand::Register(source))) =
            (operands.first(), operands.get(1))
        else {
            return Err(fail("extension operands must be registers"));
        };
        if source.view.bits() >= destination.view.bits() || width != Some(source.view) {
            return Err(fail(
                "extension requires a narrower source and source width code",
            ));
        }
    }
    if matches!(
        shape,
        Shape::DestinationSource
            | Shape::DestinationTwo
            | Shape::DestinationTwoSourceTwo
            | Shape::MemoryLoad
            | Shape::MemoryStore
            | Shape::Select
            | Shape::Address
            | Shape::CompareSwap
    ) && width.is_none()
    {
        return Err(fail("opcode requires a width"));
    }
    if matches!(shape, Shape::MemoryLoad | Shape::MemoryStore) {
        let required = match width {
            Some(View::B) => View::B,
            Some(View::W) => View::W,
            Some(View::D) => View::D,
            Some(View::Q) => View::Q,
            None => return Err(fail("memory opcode requires a width")),
        };
        let data = if shape == Shape::MemoryLoad {
            operands.first()
        } else {
            operands.get(1)
        };
        if !matches!(data, Some(VerifiedOperand::Register(register)) if register.view == required) {
            return Err(fail("memory opcode register view does not match width"));
        }
    }
    Ok(())
}

fn parse_sections<'a>(
    image: &'a [u8],
) -> Result<(BTreeMap<u32, Section<'a>>, u32, u16), VerifyError> {
    if image.len() < HEADER_SIZE {
        return Err(fail("truncated image header"));
    }
    if image.len() > MAX_IMAGE_SIZE {
        return Err(fail("image exceeds v1 format ceiling"));
    }
    if &image[0..4] != b"A+BC" {
        return Err(fail("invalid image magic"));
    }
    let major = read_u16(image, 4)?;
    let minor = read_u16(image, 6)?;
    if major != 1 || minor > 1 {
        return Err(fail("unsupported bytecode version"));
    }
    if read_u16(image, 8)? != HEADER_SIZE as u16 {
        return Err(fail("invalid header size"));
    }
    if read_u16(image, 10)? != 0 || read_u32(image, 36)? != 0 {
        return Err(fail("reserved header fields must be zero"));
    }
    if read_u64(image, 12)? != image.len() as u64 {
        return Err(fail("declared file size does not match image"));
    }
    let directory_offset = read_u64(image, 20)?;
    let count = usize::try_from(read_u32(image, 28)?)
        .map_err(|_| fail("section count is not representable"))?;
    if count > MAX_SECTIONS {
        return Err(fail("section count exceeds v1 format ceiling"));
    }
    let directory_length = count
        .checked_mul(SECTION_ENTRY_SIZE)
        .ok_or_else(|| fail("section directory size overflow"))?;
    let directory = bounded_range(
        directory_offset,
        directory_length as u64,
        image.len(),
        "section directory",
    )?;
    if directory.start < HEADER_SIZE {
        return Err(fail("section directory overlaps image header"));
    }
    let entry_function = read_u32(image, 32)? as usize;
    let mut sections = BTreeMap::new();
    let mut ranges = vec![directory.clone(), 0..HEADER_SIZE];
    for index in 0..count {
        let start = directory
            .start
            .checked_add(
                index
                    .checked_mul(SECTION_ENTRY_SIZE)
                    .ok_or_else(|| fail("directory index overflow"))?,
            )
            .ok_or_else(|| fail("directory entry overflow"))?;
        let kind = read_u32(image, start)?;
        let flags = read_u32(image, start + 4)?;
        let offset = read_u64(image, start + 8)?;
        let length = read_u64(image, start + 16)?;
        let alignment = read_u32(image, start + 24)?;
        let checksum = read_u32(image, start + 28)?;
        if flags & !SECTION_FLAG_REQUIRED != 0 {
            return Err(fail("unknown required section flags"));
        }
        if flags & SECTION_FLAG_REQUIRED != 0 && kind != SECTION_BLOCKS {
            return Err(fail("unknown required section"));
        }
        if kind == SECTION_BLOCKS && flags != SECTION_FLAG_REQUIRED {
            return Err(fail("BLOCKS section must be marked required"));
        }
        if alignment == 0 || !alignment.is_power_of_two() {
            return Err(fail("invalid section alignment"));
        }
        if offset % u64::from(alignment) != 0 {
            return Err(fail("section offset violates required alignment"));
        }
        let range = bounded_range(offset, length, image.len(), "section")?;
        if ranges
            .iter()
            .any(|existing| range.start < existing.end && existing.start < range.end)
        {
            return Err(fail(
                "section overlaps header, directory, or another section",
            ));
        }
        let data = &image[range.clone()];
        if crc32c(data) != checksum {
            return Err(fail(format!("section {kind} checksum mismatch")));
        }
        if sections
            .insert(
                kind,
                Section {
                    kind,
                    flags,
                    offset: range.start,
                    data,
                },
            )
            .is_some()
        {
            return Err(fail(format!("duplicate section type {kind}")));
        }
        ranges.push(range);
    }
    for required in [SECTION_DATA, SECTION_CODE, SECTION_CAPS] {
        if !sections.contains_key(&required) {
            return Err(fail(format!("required section {required} is missing")));
        }
    }
    if minor == 0 && sections.contains_key(&SECTION_BLOCKS) {
        return Err(fail("BLOCKS section requires bytecode minor version 1"));
    }
    Ok((sections, entry_function as u32, minor))
}

pub fn decode_image(image: &[u8]) -> Result<DecodedImage, VerifyError> {
    let (sections, raw_entry, minor_version) = parse_sections(image)?;
    let code = sections
        .get(&SECTION_CODE)
        .expect("required section checked")
        .data;
    if code.len() < 4 {
        return Err(fail("truncated CODE function count"));
    }
    let function_count = usize::try_from(read_u32(code, 0)?)
        .map_err(|_| fail("function count is not representable"))?;
    if function_count > MAX_FUNCTIONS {
        return Err(fail("function count exceeds v1 format ceiling"));
    }
    let descriptors_end = 4usize
        .checked_add(
            function_count
                .checked_mul(32)
                .ok_or_else(|| fail("function descriptor table overflow"))?,
        )
        .ok_or_else(|| fail("function descriptor table overflow"))?;
    if descriptors_end > code.len() {
        return Err(fail("truncated function descriptor table"));
    }
    let entry_function = if raw_entry == u32::MAX {
        None
    } else {
        let id = raw_entry;
        if id as usize >= function_count {
            return Err(fail("entry-function ID is outside function table"));
        }
        Some(id)
    };
    if function_count == 0 && entry_function.is_some() {
        return Err(fail("empty CODE section cannot have an entry function"));
    }
    let symbols = match sections.get(&SECTION_SYMS) {
        Some(section) => validate_symbols(section.data, function_count)?,
        None => Vec::new(),
    };
    let block_records = if let Some(block_section) = sections.get(&SECTION_BLOCKS) {
        parse_block_map(block_section.data)?
    } else {
        Vec::new()
    };
    let mut functions = Vec::with_capacity(function_count);
    let mut function_ranges = Vec::with_capacity(function_count);
    let mut unresolved_targets = Vec::new();
    for index in 0..function_count {
        let descriptor = 4 + index * 32;
        let id = read_u32(code, descriptor)?;
        let flags = read_u32(code, descriptor + 4)?;
        let code_offset = read_u64(code, descriptor + 8)?;
        let code_length = read_u64(code, descriptor + 16)?;
        let stack_frame_maximum = read_u32(code, descriptor + 24)?;
        let argument_count = read_u16(code, descriptor + 28)?;
        let result_count = read_u16(code, descriptor + 30)?;
        if id as usize != index {
            return Err(fail("function IDs must be dense and ordered"));
        }
        if flags != 0 {
            return Err(fail("unknown function flags"));
        }
        if argument_count > 8 || result_count > 2 {
            return Err(fail("function ABI shape exceeds v1 limits"));
        }
        if code_length > MAX_FUNCTION_CODE_SIZE as u64 {
            return Err(fail("function code exceeds v1 format ceiling"));
        }
        let range = bounded_range(code_offset, code_length, code.len(), "function code")?;
        if range.start < descriptors_end {
            return Err(fail("function code overlaps descriptor table"));
        }
        if function_ranges
            .iter()
            .any(|existing: &std::ops::Range<usize>| {
                range.start < existing.end && existing.start < range.end
            })
        {
            return Err(fail("function code ranges overlap"));
        }
        function_ranges.push(range.clone());
        let (instructions, targets) = decode_instruction_stream(
            &code[range.clone()],
            sections[&SECTION_CODE].offset + range.start,
        )?;
        if minor_version == 0 {
            unresolved_targets.extend(targets.iter().copied());
        }
        let mut blocks = BTreeMap::new();
        let mut mapped_offsets = BTreeSet::new();
        for (function_id, block_id, block_offset) in block_records.iter().copied() {
            if function_id != id {
                continue;
            }
            let block_offset = usize::try_from(block_offset)
                .map_err(|_| fail("block offset is not representable"))?;
            let instruction_index = instructions
                .iter()
                .position(|instruction| {
                    instruction.code_offset
                        == sections[&SECTION_CODE].offset + range.start + block_offset
                })
                .ok_or_else(|| fail("block target does not point to an instruction boundary"))?;
            if blocks
                .insert(u64::from(block_id), instruction_index)
                .is_some()
            {
                return Err(fail("duplicate block ID in function"));
            }
            if !mapped_offsets.insert(block_offset as u64) {
                return Err(fail("multiple block IDs map to the same instruction"));
            }
        }
        if minor_version == 1
            && sections.contains_key(&SECTION_BLOCKS)
            && !instructions.is_empty()
            && blocks.get(&0) != Some(&0)
        {
            return Err(fail("entry block zero must map to the first instruction"));
        }
        for target in &targets {
            if !blocks.contains_key(target) {
                return Err(fail(format!("branch target block {target} is not mapped")));
            }
        }
        functions.push(VerifiedFunction {
            id,
            flags,
            code_offset: range.start,
            code_length: range.len(),
            stack_frame_maximum,
            argument_count,
            result_count,
            instructions,
            blocks,
        });
    }
    if block_records
        .iter()
        .any(|(function_id, _, _)| *function_id as usize >= function_count)
    {
        return Err(fail("block map function ID is outside function table"));
    }
    if let Some(excp_section) = sections.get(&SECTION_EXCP) {
        validate_excp_section(excp_section.data, &functions)?;
    }
    if let Some(data_section) = sections.get(&SECTION_DATA) {
        validate_data_section(data_section.data)?;
    }
    validate_handler_balance(&functions)?;
    Ok(DecodedImage {
        minor_version,
        entry_function,
        functions,
        symbols,
        sections: sections
            .into_iter()
            .map(|(kind, section)| (kind, section.data.to_vec()))
            .collect(),
        unresolved_control_targets: unresolved_targets,
    })
}

pub fn verify_image(image: &[u8]) -> Result<DecodedImage, VerifyError> {
    let decoded = decode_image(image)?;
    if !decoded.unresolved_control_targets.is_empty() {
        return Err(fail(
            "block target offsets are not represented in this image format; control-flow image cannot be execution-verified",
        ));
    }
    validate_capabilities(
        decoded.sections.get(&SECTION_CAPS).map(|v| v.as_slice()),
        &decoded.functions,
    )?;
    Ok(decoded)
}

fn parse_block_map(bytes: &[u8]) -> Result<Vec<(u32, u32, u64)>, VerifyError> {
    if bytes.len() < 4 {
        return Err(fail("truncated BLOCKS section"));
    }
    let count = usize::try_from(read_u32(bytes, 0)?)
        .map_err(|_| fail("block map record count is not representable"))?;
    let expected = 4usize
        .checked_add(
            count
                .checked_mul(16)
                .ok_or_else(|| fail("block map size overflow"))?,
        )
        .ok_or_else(|| fail("block map size overflow"))?;
    if expected != bytes.len() {
        return Err(fail("BLOCKS section length does not match record count"));
    }
    let mut records = Vec::with_capacity(count);
    let mut keys = BTreeSet::new();
    for index in 0..count {
        let offset = 4 + index * 16;
        let function_id = read_u32(bytes, offset)?;
        let block_id = read_u32(bytes, offset + 4)?;
        let code_offset = read_u64(bytes, offset + 8)?;
        if !keys.insert((function_id, block_id)) {
            return Err(fail("duplicate function/block mapping"));
        }
        records.push((function_id, block_id, code_offset));
    }
    Ok(records)
}

fn validate_symbols(bytes: &[u8], function_count: usize) -> Result<Vec<Symbol>, VerifyError> {
    if bytes.len() < 4 {
        return Err(fail("truncated SYMS section"));
    }
    let count = read_u32(bytes, 0)? as usize;
    let mut position = 4usize;
    let mut names = BTreeSet::new();
    let mut symbols = Vec::with_capacity(count);
    for _ in 0..count {
        let length = usize::from(read_u16(bytes, position)?);
        position = position
            .checked_add(2)
            .ok_or_else(|| fail("symbol offset overflow"))?;
        let end = position
            .checked_add(length)
            .ok_or_else(|| fail("symbol name length overflow"))?;
        let name = bytes
            .get(position..end)
            .ok_or_else(|| fail("truncated symbol name"))?;
        if !name.is_ascii() {
            return Err(fail("symbol name must be ASCII"));
        }
        if !names.insert(name) {
            return Err(fail("duplicate symbol name"));
        }
        position = end;
        let function_id = read_u32(bytes, position)? as usize;
        position = position
            .checked_add(4)
            .ok_or_else(|| fail("symbol record overflow"))?;
        if function_id >= function_count {
            return Err(fail("symbol function ID is outside function table"));
        }
        let name =
            String::from_utf8(name.to_vec()).map_err(|_| fail("symbol name must be ASCII"))?;
        symbols.push(Symbol {
            name,
            function: function_id as u32,
        });
    }
    if position != bytes.len() {
        return Err(fail("trailing bytes in SYMS section"));
    }
    Ok(symbols)
}

fn validate_excp_section(
    bytes: &[u8],
    functions: &[crate::verifier::VerifiedFunction],
) -> Result<(), VerifyError> {
    if bytes.len() < 4 {
        return Err(fail("truncated EXCP section"));
    }
    let count = usize::try_from(read_u32(bytes, 0)?)
        .map_err(|_| fail("exception record count is not representable"))?;
    let expected = 4usize
        .checked_add(
            count
                .checked_mul(16)
                .ok_or_else(|| fail("exception section size overflow"))?,
        )
        .ok_or_else(|| fail("exception section size overflow"))?;
    if expected != bytes.len() {
        return Err(fail("EXCP section length does not match record count"));
    }
    let mut ranges: Vec<(u32, u32, u32, u32)> = Vec::with_capacity(count);
    for index in 0..count {
        let offset = 4 + index * 16;
        let function_id = read_u32(bytes, offset)?;
        let start_block = read_u32(bytes, offset + 4)?;
        let end_block = read_u32(bytes, offset + 8)?;
        let handler_block = read_u32(bytes, offset + 12)?;
        if function_id as usize >= functions.len() {
            return Err(fail(
                "exception handler function ID is outside function table",
            ));
        }
        let function = &functions[function_id as usize];
        let block_exists = |block_id: u32| function.blocks.contains_key(&u64::from(block_id));
        if !block_exists(start_block) || !block_exists(end_block) || !block_exists(handler_block) {
            return Err(fail(
                "exception handler block is not mapped in its function",
            ));
        }
        if start_block >= end_block {
            return Err(fail("exception protected range is empty or inverted"));
        }
        if handler_block >= start_block && handler_block < end_block {
            return Err(fail("exception handler lies inside its protected range"));
        }
        ranges.push((function_id, start_block, end_block, handler_block));
    }
    for i in 0..ranges.len() {
        for j in i + 1..ranges.len() {
            if ranges[i].0 != ranges[j].0 {
                continue;
            }
            let a = &ranges[i];
            let b = &ranges[j];
            if a.1 < b.2 && b.1 < a.2 {
                return Err(fail("exception protected ranges overlap"));
            }
        }
    }
    Ok(())
}

fn validate_data_section(bytes: &[u8]) -> Result<(), VerifyError> {
    if bytes.is_empty() {
        return Ok(());
    }
    if bytes.len() < 4 {
        return Err(fail("truncated DATA section"));
    }
    let count = usize::try_from(read_u32(bytes, 0)?)
        .map_err(|_| fail("data record count is not representable"))?;
    let expected = 4usize
        .checked_add(
            count
                .checked_mul(24)
                .ok_or_else(|| fail("data section size overflow"))?,
        )
        .ok_or_else(|| fail("data section size overflow"))?;
    if expected != bytes.len() {
        return Err(fail("DATA section length does not match record count"));
    }
    for index in 0..count {
        let offset = 4 + index * 24;
        let _region_id = read_u32(bytes, offset)?;
        let data_offset = read_u64(bytes, offset + 4)?;
        let length = read_u64(bytes, offset + 12)?;
        let _flags = read_u32(bytes, offset + 20)?;
        let _end = data_offset
            .checked_add(length)
            .ok_or_else(|| fail("data record range overflow"))?;
    }
    Ok(())
}

fn validate_handler_balance(functions: &[VerifiedFunction]) -> Result<(), VerifyError> {
    for function in functions {
        let mut depth: i32 = 0;
        for instruction in &function.instructions {
            match instruction.opcode {
                0x0007 => depth += 1,
                0x0008 => {
                    depth -= 1;
                    if depth < 0 {
                        return Err(fail(format!(
                            "handler stack underflow in function {}",
                            function.id
                        )));
                    }
                }
                _ => {}
            }
        }
    }
    Ok(())
}

fn validate_capabilities(
    caps_section: Option<&[u8]>,
    functions: &[VerifiedFunction],
) -> Result<(), VerifyError> {
    let mut granted: u32 = 0;
    if let Some(data) = caps_section
        && !data.is_empty()
    {
        if data.len() < 4 {
            return Err(fail("truncated CAPS section"));
        }
        let count = u32::from_le_bytes([data[0], data[1], data[2], data[3]]) as usize;
        let expected = 4usize
            .checked_add(
                count
                    .checked_mul(4)
                    .ok_or_else(|| fail("CAPS section size overflow"))?,
            )
            .ok_or_else(|| fail("CAPS section size overflow"))?;
        if expected != data.len() {
            return Err(fail("CAPS section length does not match record count"));
        }
        for index in 0..count {
            let offset = 4 + index * 4;
            let cap = u32::from_le_bytes([
                data[offset],
                data[offset + 1],
                data[offset + 2],
                data[offset + 3],
            ]);
            granted |= cap;
        }
    }
    for function in functions {
        for instruction in &function.instructions {
            let required = match instruction.opcode {
                0x0057 | 0x0058 => CAP_MEMORY_MAP,
                0x0059 | 0x005A => CAP_IO_PORT,
                0x005B..=0x005D => CAP_IRQ_MANAGE,
                0x005E | 0x005F => CAP_CONTEXT_SWITCH,
                0x0067 => CAP_CPU_CONTROL,
                0x0068 => CAP_POWER_CONTROL,
                _ => continue,
            };
            if granted & required == 0 {
                return Err(fail(format!(
                    "function {} uses sys.* without required capability",
                    function.id
                )));
            }
        }
    }
    Ok(())
}
