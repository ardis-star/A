use std::fmt;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::model::View;
use crate::verifier::{
    VerifiedFunction, VerifiedInstruction, VerifiedOperand, VerifiedRegister, verify_image,
};

const FLAG_DIRECTION: u64 = 1 << 5;
/// Offset of the architectural FLAGS word inside the native frame.
const FLAGS_OFFSET: i32 = -264;
/// Total frame size: 32 registers, FLAGS, return slot, plus alignment padding.
const FRAME_SIZE: usize = 304;
/// Exported symbol holding the fault code recorded by generated code, so a
/// caller (the JIT loader) can tell a fault from a normal result.
pub const FAULT_SYMBOL: &str = "ascension_fault_code";
/// Frame slot holding a non-zero marker when the program has faulted.
const FAULT_FLAG_OFFSET: i32 = -280;
/// Value stored in [`FAULT_FLAG_OFFSET`] once the program has faulted.
const FAULT_FLAG: i64 = 1;
/// Frame slot holding the fault code (`0` when the program did not fault).
const FAULT_CODE_OFFSET: i32 = -288;
/// Exit status a native executable returns when the program faulted at
/// runtime. Chosen to sit outside the range the toolchain itself uses.
pub const NATIVE_FAULT_EXIT: i32 = 70;
/// Fault codes recorded in [`FAULT_SYMBOL`].
pub const FAULT_DIVIDE_BY_ZERO: i32 = 1;

/// Opcodes the native backends (JIT and AOT) can lower, as inclusive ranges.
///
/// This is the single source of truth for the native subset: `emit_instruction`
/// refuses anything outside it, [`native_supports`] answers queries about it,
/// and the public documentation table is checked against it by a test.
pub const NATIVE_OPCODE_RANGES: &[(u16, u16)] = &[
    (0x0001, 0x0001), // nop
    (0x0003, 0x0006), // return, stop, div.u, div.s
    (0x000A, 0x000D), // flag.direction, fence, compare.swap
    (0x0010, 0x0010), // move
    (0x0020, 0x0024), // sum, sum!, diff, diff!, compare!
    (0x002A, 0x002A), // mul!
    (0x0030, 0x0035), // and/or/xor/not, with and without flags
    (0x0050, 0x0056), // jump, if.*, call, tailcall
    (0x0060, 0x0066), // exchange, if.sign/notsign/overflow/nooverflow/even/odd
];

/// Whether the native backends can lower `opcode`.
pub fn native_supports(opcode: u16) -> bool {
    NATIVE_OPCODE_RANGES
        .iter()
        .any(|(first, last)| (*first..=*last).contains(&opcode))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeError(pub String);

impl fmt::Display for NativeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for NativeError {}

fn error(message: impl Into<String>) -> NativeError {
    NativeError(message.into())
}

fn register_offset(register: VerifiedRegister) -> i32 {
    -8 * (i32::from(register.index) + 1)
}

fn offset_text(offset: i32, base: &str) -> String {
    format!("{offset}({base})")
}

fn view_mask(view: View) -> u64 {
    match view {
        View::B => 0xff,
        View::W => 0xffff,
        View::D => 0xffff_ffff,
        View::Q => u64::MAX,
    }
}

fn width_suffix(view: View) -> &'static str {
    match view {
        View::B => "b",
        View::W => "w",
        View::D => "l",
        View::Q => "q",
    }
}

fn accumulator_register(view: View) -> &'static str {
    match view {
        View::B => "%al",
        View::W => "%ax",
        View::D => "%eax",
        View::Q => "%rax",
    }
}

fn data_register(view: View) -> &'static str {
    match view {
        View::B => "%dl",
        View::W => "%dx",
        View::D => "%edx",
        View::Q => "%rdx",
    }
}

/// Mask `destination` down to `mask` bits, zero-extending the result.
///
/// Every form used here is unconditionally encodable: `andl` with a value below
/// 2^32-1, a self-move of the 32-bit register for a full 32-bit mask, and a
/// register-held mask for anything wider.
fn emit_mask(destination: &str, mask: u64) -> String {
    if mask == u64::MAX {
        return String::new();
    }
    let low = source_32(destination);
    if mask < u64::from(u32::MAX) {
        format!("\tandl ${mask}, {low}\n")
    } else if mask == u64::from(u32::MAX) {
        format!("\tmovl {low}, {low}\n")
    } else {
        format!(
            "\tmovabsq ${}, %r11\n\tandq %r11, {destination}\n",
            mask as i64
        )
    }
}

fn load_scalar(
    instruction: &VerifiedInstruction,
    operand_index: usize,
    width: View,
    destination: &str,
) -> Result<String, NativeError> {
    let operand = instruction
        .operands
        .get(operand_index)
        .ok_or_else(|| error("missing operand while lowering native instruction"))?;
    let mask = view_mask(width);
    let mut assembly = String::new();
    match operand {
        VerifiedOperand::Register(register) => {
            assembly.push_str(&format!(
                "\tmovq {}, {destination}\n",
                offset_text(register_offset(*register), "%rbp")
            ));
            if mask != u64::MAX {
                assembly.push_str(&emit_mask(destination, mask));
            }
        }
        // Unsigned immediates always fit the instruction width (the verifier
        // enforces it), so no mask is required.
        VerifiedOperand::Unsigned(value) => {
            assembly.push_str(&format!("\tmovabsq $0x{value:016x}, {destination}\n"));
        }
        // A negative immediate is a two's-complement value for this width; the
        // mask turns it back into the raw bit pattern without any `and` over a
        // value the assembler would reject.
        VerifiedOperand::Signed(value) => {
            let raw = (*value as u64) & mask;
            assembly.push_str(&format!("\tmovabsq $0x{raw:016x}, {destination}\n"));
        }
        _ => {
            return Err(error(
                "native subset requires register or immediate scalar operands",
            ));
        }
    }
    Ok(assembly)
}

/// Store a computed value in a virtual register, honouring sub-register views:
/// `.b` / `.w` preserve the upper bits, `.d` zero-extends into the full slot,
/// `.q` replaces it. The value is staged in `%rax`.
fn write_register(register: VerifiedRegister, source: &str) -> String {
    let target = offset_text(register_offset(register), "%rbp");
    let mut assembly = String::new();
    if source != "%rax" {
        assembly.push_str(&format!("\tmovq {source}, %rax\n"));
    }
    match register.view {
        View::Q => assembly.push_str(&format!("\tmovq %rax, {target}\n")),
        // `.d` writes replace the whole slot and zero-extend.
        View::D => assembly.push_str(&format!("\tmovl %eax, %eax\n\tmovq %rax, {target}\n")),
        View::B | View::W => {
            // Keep the slot's upper bits and replace the low byte/word with the
            // zero-extended low bits of `%rax`.
            let keep_upper = if register.view == View::B {
                "andq $-256, %rdx"
            } else {
                "andq $-65536, %rdx"
            };
            let narrow = if register.view == View::B {
                "movzbq %al, %rax"
            } else {
                "movzwq %ax, %rax"
            };
            assembly.push_str(&format!(
                "\tmovq {target}, %rdx\n\t{keep_upper}\n\t{narrow}\n\torq %rax, %rdx\n\tmovq %rdx, {target}\n"
            ));
        }
    }
    assembly
}

/// Low byte/word name of a 64-bit scratch register.
fn narrow_low(register: &str, suffix: &str) -> String {
    match (source_32(register), suffix) {
        ("%eax", "b") => "%al".to_string(),
        ("%eax", _) => "%ax".to_string(),
        ("%edx", "b") => "%dl".to_string(),
        ("%edx", _) => "%dx".to_string(),
        ("%ecx", "b") => "%cl".to_string(),
        ("%ecx", _) => "%cx".to_string(),
        (other, "b") => format!("{}l", &other[..2]),
        (other, _) => format!("{}x", &other[..2]),
    }
}

/// 32-bit name of the 64-bit scratch registers this backend uses.
fn source_32(source: &str) -> &'static str {
    match source {
        "%rdx" => "%edx",
        "%rcx" => "%ecx",
        "%rbx" => "%ebx",
        _ => "%eax",
    }
}

fn capture_flags() -> String {
    let mut assembly = String::new();
    assembly.push_str("\tsetc %r8b\n\tseto %r9b\n\tsetz %r10b\n\tsets %r11b\n\tsetp %cl\n");
    assembly.push_str(
        "\tmovzbq %r8b, %r8\n\tmovzbq %r9b, %r9\n\tmovzbq %r10b, %r10\n\tmovzbq %r11b, %r11\n\tmovzbq %cl, %rcx\n\tshlq $1, %r8\n\tshlq $3, %r9\n\tshlq $2, %r11\n\tshlq $4, %rcx\n\torq %r8, %r9\n\torq %r10, %r9\n\torq %r11, %r9\n\torq %rcx, %r9\n",
    );
    assembly.push_str(&format!(
        "\tmovq {}, %rdx\n\tandq ${FLAG_DIRECTION}, %rdx\n\torq %rdx, %r9\n\tmovq %r9, {}\n",
        offset_text(FLAGS_OFFSET, "%rbp"),
        offset_text(FLAGS_OFFSET, "%rbp")
    ));
    assembly
}

fn emit_move(instruction: &VerifiedInstruction) -> Result<String, NativeError> {
    let Some(VerifiedOperand::Register(destination)) = instruction.operands.first() else {
        return Err(error("invalid move destination"));
    };
    let width = instruction
        .width
        .ok_or_else(|| error("move has no width"))?;
    let mut assembly = load_scalar(instruction, 1, width, "%rax")?;
    assembly.push_str(&write_register(*destination, "%rax"));
    Ok(assembly)
}

fn emit_alu(instruction: &VerifiedInstruction) -> Result<String, NativeError> {
    let opcode = instruction.opcode;
    let width = instruction
        .width
        .ok_or_else(|| error("ALU instruction has no width"))?;
    let compare = opcode == 0x0024;
    let (destination, left_index, right_index) = if compare {
        (None, 0, 1)
    } else {
        let Some(VerifiedOperand::Register(register)) = instruction.operands.first() else {
            return Err(error("native ALU destination must be a register"));
        };
        (Some(*register), 1, 2)
    };
    let mut assembly = load_scalar(instruction, left_index, width, "%rax")?;
    assembly.push_str(&load_scalar(instruction, right_index, width, "%rdx")?);
    let suffix = width_suffix(width);
    // `imul` has no byte-immediate form; imulw/imull/imulq are the only
    // two-operand versions, so narrow multiplication is done at 32 bits and the
    // result is masked back to the instruction width afterwards.
    let multiply_suffix = if width == View::B { "l" } else { suffix };
    let multiply_accumulator = if width == View::B {
        "%eax".to_string()
    } else {
        accumulator_register(width).to_string()
    };
    let multiply_data = if width == View::B {
        "%edx".to_string()
    } else {
        data_register(width).to_string()
    };
    let operation = match opcode {
        0x0020 | 0x0021 => format!(
            "\tadd{suffix} {}, {}\n",
            data_register(width),
            accumulator_register(width)
        ),
        0x0022..=0x0024 => format!(
            "\tsub{suffix} {}, {}\n",
            data_register(width),
            accumulator_register(width)
        ),
        0x002a => format!("\timul{multiply_suffix} {multiply_data}, {multiply_accumulator}\n"),
        0x0030 | 0x0031 => format!(
            "\tand{suffix} {}, {}\n",
            data_register(width),
            accumulator_register(width)
        ),
        0x0032 | 0x0033 => format!(
            "\tor{suffix} {}, {}\n",
            data_register(width),
            accumulator_register(width)
        ),
        0x0034 | 0x0035 => format!(
            "\txor{suffix} {}, {}\n",
            data_register(width),
            accumulator_register(width)
        ),
        _ => {
            return Err(error(format!(
                "opcode 0x{opcode:04x} is outside the native AOT subset"
            )));
        }
    };
    assembly.push_str(&operation);
    let update_flags = matches!(
        opcode,
        0x0021 | 0x0023 | 0x0024 | 0x002a | 0x0031 | 0x0033 | 0x0035
    );
    if update_flags {
        let arithmetic = matches!(opcode, 0x0021 | 0x0023 | 0x0024);
        if !arithmetic {
            // `mul` is truncating and flag-setting: the architectural flags are
            // derived from the truncated product, which may be narrower than the
            // native operands, so mask and test the exact result.
            assembly.push_str(&emit_mask("%rax", view_mask(width)));
            assembly.push_str(&format!(
                "\ttest{suffix} {}, {}\n",
                accumulator_register(width),
                accumulator_register(width)
            ));
        }
        assembly.push_str(&capture_flags());
    }
    if let Some(destination) = destination {
        assembly.push_str(&write_register(destination, "%rax"));
    }
    Ok(assembly)
}

fn block_label(function: &VerifiedFunction, block_id: u64) -> Result<String, NativeError> {
    if !function.blocks.contains_key(&block_id) {
        return Err(error(format!("unresolved block target {block_id}")));
    }
    Ok(format!(".Lpc{}", function.blocks[&block_id]))
}

fn emit_branch(
    instruction: &VerifiedInstruction,
    function: &VerifiedFunction,
) -> Result<String, NativeError> {
    let Some(VerifiedOperand::Block(block_id)) = instruction.operands.first() else {
        return Err(error("branch has no block operand"));
    };
    let target = block_label(function, *block_id)?;
    let mut assembly = String::new();
    match instruction.opcode {
        0x0050 => assembly.push_str(&format!("\tjmp {target}\n")),
        // `call` and `return` use the machine stack directly (see the entry
        // sequence, which pushes a sentinel so a top-level `return` exits).
        0x0055 => assembly.push_str(&format!("\tcallq {target}\n")),
        0x0056 => assembly.push_str(&format!("\tjmp {target}\n")),
        0x0051..=0x0054 | 0x0061..=0x0066 => {
            let flag = match instruction.opcode {
                0x0051 | 0x0052 => 0,
                0x0053 | 0x0054 => 1,
                0x0061 | 0x0062 => 2,
                0x0063 | 0x0064 => 3,
                0x0065 | 0x0066 => 4,
                _ => unreachable!(),
            };
            let set = matches!(
                instruction.opcode,
                0x0051 | 0x0053 | 0x0061 | 0x0063 | 0x0065
            );
            assembly.push_str(&format!(
                "\tbtq ${flag}, {}\n\t{} {target}\n",
                offset_text(FLAGS_OFFSET, "%rbp"),
                if set { "jc" } else { "jnc" }
            ));
        }
        _ => {
            return Err(error(format!(
                "opcode 0x{:04x} is outside the native AOT subset",
                instruction.opcode
            )));
        }
    }
    Ok(assembly)
}

fn emit_exchange(instruction: &VerifiedInstruction) -> Result<String, NativeError> {
    let (Some(VerifiedOperand::Register(left)), Some(VerifiedOperand::Register(right))) =
        (instruction.operands.first(), instruction.operands.get(1))
    else {
        return Err(error("exchange operands must be registers"));
    };
    Ok(format!(
        "\tmovq {}, %rax\n\tmovq {}, %rdx\n\tmovq %rax, {}\n\tmovq %rdx, {}\n",
        offset_text(register_offset(*left), "%rbp"),
        offset_text(register_offset(*right), "%rbp"),
        offset_text(register_offset(*right), "%rbp"),
        offset_text(register_offset(*left), "%rbp"),
    ))
}

fn emit_div(instruction: &VerifiedInstruction) -> Result<String, NativeError> {
    let opcode = instruction.opcode;
    let width = instruction.width.ok_or_else(|| error("div has no width"))?;
    let dest_q = instruction
        .operands
        .first()
        .and_then(|o| match o {
            VerifiedOperand::Register(r) => Some(*r),
            _ => None,
        })
        .ok_or_else(|| error("div needs destination quotient register"))?;
    let dest_m = instruction
        .operands
        .get(1)
        .and_then(|o| match o {
            VerifiedOperand::Register(r) => Some(*r),
            _ => None,
        })
        .ok_or_else(|| error("div needs destination remainder register"))?;
    let signed = opcode == 0x0006;
    let mask = view_mask(width);
    let load = if signed {
        load_signed_scalar
    } else {
        load_scalar
    };
    let mut assembly = load(instruction, 2, width, "%rax")?;
    assembly.push_str(&load(instruction, 3, width, "%rcx")?);
    // A zero divisor raises #DE on the host; report it as the VM fault instead
    // of letting the process die with SIGFPE.
    let continue_label = format!(".Ldivok{}", instruction.code_offset);
    assembly.push_str(&format!("\ttestq %rcx, %rcx\n\tjnz {continue_label}\n"));
    assembly.push_str(&format!(
        "\tmovq ${FAULT_DIVIDE_BY_ZERO}, {}\n\tjmp .Lfault\n{continue_label}:\n",
        offset_text(FAULT_CODE_OFFSET, "%rbp")
    ));
    if signed {
        // The native idiv is the full 64-bit form. Narrow signed operands are
        // produced already sign-extended from their own width into 64 bits, so
        // the full-width division yields exactly the AVM's width-masked result.
        assembly.push_str("\tcqto\n\tidivq %rcx\n");
    } else {
        // Unsigned operands are zero-extended, so a 64-bit divide is exact.
        assembly.push_str("\txorq %rdx, %rdx\n\tdivq %rcx\n");
    }
    // Both results are narrowed to the operation width. The remainder moves to
    // %r10 because the sub-register store uses %rdx as scratch.
    assembly.push_str(&emit_mask("%rax", mask));
    assembly.push_str(&emit_mask("%rdx", mask));
    assembly.push_str("\tmovq %rdx, %r10\n");
    assembly.push_str(&write_register(dest_q, "%rax"));
    assembly.push_str(&write_register(dest_m, "%r10"));
    Ok(assembly)
}

/// A negative narrow operand must reach the native divide already
/// sign-extended from its own width; `load_scalar` masks, so division and
/// arithmetic-shift lowering widen explicitly here.
fn load_signed_scalar(
    instruction: &VerifiedInstruction,
    operand_index: usize,
    width: View,
    destination: &str,
) -> Result<String, NativeError> {
    let mut assembly = load_scalar(instruction, operand_index, width, destination)?;
    let (operation, low) = match width {
        View::B => ("movsbq", narrow_low(destination, "b")),
        View::W => ("movswq", narrow_low(destination, "w")),
        View::D => ("movslq", destination.to_string()),
        View::Q => return Ok(assembly),
    };
    assembly.push_str(&format!("\t{operation} {low}, {destination}\n"));
    Ok(assembly)
}

fn emit_flag_direction(instruction: &VerifiedInstruction) -> Result<String, NativeError> {
    let value = match instruction.operands.first() {
        Some(VerifiedOperand::Unsigned(v)) => *v != 0,
        Some(VerifiedOperand::Signed(v)) => *v != 0,
        _ => return Err(error("flag.direction requires a mode")),
    };
    if value {
        Ok("\tstd\n".to_owned())
    } else {
        Ok("\tcld\n".to_owned())
    }
}

fn emit_fence(instruction: &VerifiedInstruction) -> Result<String, NativeError> {
    let mode = match instruction.operands.first() {
        Some(VerifiedOperand::Unsigned(v)) => *v,
        Some(VerifiedOperand::Signed(v)) => *v as u64,
        _ => return Err(error("fence requires a mode")),
    };
    let instr = match mode {
        0 => "lfence", // acquire
        1 => "sfence", // release
        2 => "mfence", // full
        _ => {
            return Err(error(
                "fence requires mode 0 (acquire), 1 (release), or 2 (full)",
            ));
        }
    };
    Ok(format!("\t{instr}\n"))
}

fn emit_compare_swap(instruction: &VerifiedInstruction) -> Result<String, NativeError> {
    let width = instruction
        .width
        .ok_or_else(|| error("compare.swap has no width"))?;
    let dest = instruction
        .operands
        .first()
        .and_then(|o| match o {
            VerifiedOperand::Register(r) => Some(*r),
            _ => None,
        })
        .ok_or_else(|| error("compare.swap needs destination register"))?;
    let mask = view_mask(width);
    let mut assembly = load_scalar(instruction, 2, width, "%rax")?; // expected -> %rax
    assembly.push_str(&load_scalar(instruction, 3, width, "%rbx")?); // desired -> %rbx
    // Address: the base register's full 64-bit value plus the encoded
    // displacement. `cmpxchg` only has 32- and 64-bit forms, so narrower
    // widths are performed as 32-bit operations on an aligned dword.
    let (base, offset) = match instruction.operands.get(1) {
        Some(VerifiedOperand::Memory { base, offset }) => (*base, *offset),
        _ => return Err(error("compare.swap needs memory operand")),
    };
    assembly.push_str(&format!(
        "\tmovq {}, %rcx\n",
        offset_text(register_offset(base), "%rbp")
    ));
    if offset != 0 {
        assembly.push_str(&format!("\taddq ${offset}, %rcx\n"));
    }
    let comparison = if width == View::Q {
        "cmpxchgq %rbx, (%rcx)"
    } else {
        "cmpxchgl %ebx, (%rcx)"
    };
    assembly.push_str(&format!("\t{comparison}\n"));
    // %rax now holds the signalled (old) value; mask it to the operation width
    // so the architectural register receives the same bits the AVM produces.
    assembly.push_str(&emit_mask("%rax", mask));
    assembly.push_str(&write_register(dest, "%rax"));
    Ok(assembly)
}

pub fn emit_assembly(image: &[u8]) -> Result<String, NativeError> {
    if !cfg!(target_arch = "x86_64") {
        return Err(error("native AOT currently requires an x86-64 host"));
    }
    let decoded =
        verify_image(image).map_err(|verify| error(format!("verification failed: {verify}")))?;
    if decoded.entry_function != Some(0) || decoded.functions.len() != 1 {
        return Err(error(
            "native AOT subset requires one entry function with ID 0",
        ));
    }
    let function = &decoded.functions[0];
    if function.instructions.is_empty() {
        return Err(error("native AOT does not accept an empty function"));
    }
    let mut assembly = String::from(
        ".text\n.globl main\n.type main,@function\nmain:\n\tpushq %rbp\n\tmovq %rsp, %rbp\n",
    );
    assembly.push_str(&format!("\tsubq ${FRAME_SIZE}, %rsp\n"));
    for index in 0..32 {
        assembly.push_str(&format!(
            "\tmovq $0, {}\n",
            offset_text(-8 * (index + 1), "%rbp")
        ));
    }
    assembly.push_str(&format!(
        "\tmovq $0, {}\n\tmovq $0, {}\n\tmovq $0, {}\n",
        offset_text(FLAGS_OFFSET, "%rbp"),
        offset_text(FAULT_FLAG_OFFSET, "%rbp"),
        offset_text(FAULT_CODE_OFFSET, "%rbp")
    ));
    let entry = function.blocks.get(&0).copied().unwrap_or(0);
    // A sentinel return point is pushed so a `return` that has no matching
    // `call` leaves through `.Lexit`, matching the interpreter, which reports
    // `returned` when the call stack is empty.
    assembly.push_str("\tleaq .Lexit(%rip), %rax\n\tpushq %rax\n");
    assembly.push_str(&format!("\tjmp .Lpc{entry}\n"));
    let blocks_by_instruction: Vec<Vec<u64>> = {
        let mut blocks = vec![Vec::new(); function.instructions.len()];
        for (block_id, instruction_index) in &function.blocks {
            if let Some(at) = blocks.get_mut(*instruction_index) {
                at.push(*block_id);
            }
        }
        blocks
    };
    for (index, instruction) in function.instructions.iter().enumerate() {
        assembly.push_str(&format!(".Lpc{index}:\n"));
        for block_id in &blocks_by_instruction[index] {
            if *block_id != 0 || index != entry {
                assembly.push_str(&format!(".Lblock{block_id}:\n"));
            }
        }
        match instruction.opcode {
            0x0001 => assembly.push_str("\tnop\n"),
            0x0003 => assembly.push_str("\tretq\n"),
            0x0004 => assembly.push_str("\tjmp .Lexit\n"),
            0x0005 | 0x0006 => assembly.push_str(&emit_div(instruction)?),
            // `address` names a location in the bytecode image, which has no
            // meaning in a native executable. Reject it rather than emit a
            // value that disagrees with the interpreter.
            0x000B => {
                return Err(error(
                    "opcode `address` is not supported by the native AOT backend: it names a bytecode offset, which native code does not use",
                ));
            }
            0x000A => assembly.push_str(&emit_flag_direction(instruction)?),
            0x000C => assembly.push_str(&emit_fence(instruction)?),
            0x000D => assembly.push_str(&emit_compare_swap(instruction)?),
            0x0010 => assembly.push_str(&emit_move(instruction)?),
            0x0020..=0x0024 | 0x002a | 0x0030..=0x0035 => {
                assembly.push_str(&emit_alu(instruction)?)
            }
            0x0060 => assembly.push_str(&emit_exchange(instruction)?),
            0x0050..=0x0056 | 0x0061..=0x0066 => {
                assembly.push_str(&emit_branch(instruction, function)?)
            }
            opcode => {
                let name = crate::instructions::mnemonic_for(opcode).unwrap_or("unknown");
                return Err(error(format!(
                    "`{name}` (opcode 0x{opcode:04x}) is not supported by the native backends; see manual/JIT-AOT.md for the supported subset"
                )));
            }
        }
    }
    // A runtime fault records a code in the exported fault word and leaves
    // through `.Lexit`, so the caller can distinguish a fault from a `stop`.
    // Faults are published through the exported word so the JIT loader (and
    // any embedder) sees the same condition the interpreter would report. The
    // address is taken through the GOT so the same assembly links both as a
    // position-independent shared object and as a static executable.
    assembly.push_str(&format!(
        ".Lfault:\n\tmovq ${FAULT_FLAG}, {}\n.Lexit:\n",
        offset_text(FAULT_FLAG_OFFSET, "%rbp")
    ));
    assembly.push_str(&format!(
        "\tmovq {FAULT_SYMBOL}@GOTPCREL(%rip), %r11\n\
         \tmovq {}, %rax\n\
         \tmovl %eax, (%r11)\n\
         \ttestq %rax, %rax\n\
         \tjne .Lfault_exit\n\
         \tmovq {}(%rbp), %rax\n\
         \tleave\n\tret\n\
         .Lfault_exit:\n\tmovl ${NATIVE_FAULT_EXIT}, %eax\n\tleave\n\tret\n\
         .size main, .-main\n.section .note.GNU-stack,\"\",@progbits\n",
        offset_text(FAULT_CODE_OFFSET, "%rbp"),
        register_offset(VerifiedRegister {
            index: 0,
            view: View::Q
        })
    ));
    assembly.push_str(&format!(
        ".bss\n.align 8\n.globl {FAULT_SYMBOL}\n.type {FAULT_SYMBOL},@object\n.size {FAULT_SYMBOL},8\n{FAULT_SYMBOL}:\n\t.zero 8\n"
    ));
    Ok(assembly)
}

pub fn compile_executable(image: &[u8], output: &Path) -> Result<(), NativeError> {
    let assembly = emit_assembly(image)?;
    let mut child = Command::new("cc")
        .args(["-x", "assembler", "-", "-o"])
        .arg(output)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| NativeError(format!("failed to start cc: {error}")))?;
    child
        .stdin
        .take()
        .ok_or_else(|| error("failed to open cc stdin"))?
        .write_all(assembly.as_bytes())
        .map_err(|error| NativeError(format!("failed to send assembly to cc: {error}")))?;
    let result = child
        .wait_with_output()
        .map_err(|error| NativeError(format!("failed to wait for cc: {error}")))?;
    if !result.status.success() {
        return Err(NativeError(format!(
            "native linker failed: {}",
            String::from_utf8_lossy(&result.stderr).trim()
        )));
    }
    Ok(())
}

/// List every instruction in `image` that the native backends cannot lower.
///
/// The report is what `+a run --native` and `+a build --native` show when they
/// refuse an image, so a caller is told which construct is missing instead of
/// being dropped back onto the interpreter. It returns `Ok` only when the whole
/// image is translatable.
pub fn unsupported_instructions(image: &[u8]) -> Result<Vec<String>, NativeError> {
    let decoded = verify_image(image).map_err(|error| NativeError(error.to_string()))?;
    let mut missing: Vec<String> = Vec::new();
    for function in &decoded.functions {
        for instruction in &function.instructions {
            if native_supports(instruction.opcode) {
                continue;
            }
            let name = crate::instructions::mnemonic_for(instruction.opcode)
                .unwrap_or("unknown")
                .to_string();
            if !missing.contains(&name) {
                missing.push(name);
            }
        }
    }
    Ok(missing)
}
