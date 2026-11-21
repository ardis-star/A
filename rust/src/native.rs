use std::fmt;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::model::View;
use crate::verifier::{
    VerifiedFunction, VerifiedInstruction, VerifiedOperand, VerifiedRegister, verify_image,
};

const FLAG_DIRECTION: u64 = 1 << 5;
const FLAGS_OFFSET: i32 = -264;
const FRAME_SIZE: usize = 288;

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
    let mut assembly = String::new();
    match operand {
        VerifiedOperand::Register(register) => {
            assembly.push_str(&format!(
                "\tmovq {}, {destination}\n",
                offset_text(register_offset(*register), "%rbp")
            ));
            let mask = view_mask(width);
            if mask != u64::MAX {
                assembly.push_str(&format!("\tandq ${mask}, {destination}\n"));
            }
        }
        VerifiedOperand::Unsigned(value) => {
            assembly.push_str(&format!("\tmovabsq $0x{value:016x}, {destination}\n"));
            let mask = view_mask(width);
            if mask != u64::MAX {
                assembly.push_str(&format!("\tandq ${mask}, {destination}\n"));
            }
        }
        VerifiedOperand::Signed(value) => {
            assembly.push_str(&format!(
                "\tmovabsq $0x{:016x}, {destination}\n",
                *value as u64
            ));
            let mask = view_mask(width);
            if mask != u64::MAX {
                assembly.push_str(&format!("\tandq ${mask}, {destination}\n"));
            }
        }
        _ => {
            return Err(error(
                "native subset requires register or immediate scalar operands",
            ));
        }
    }
    Ok(assembly)
}

fn write_register(register: VerifiedRegister, source: &str) -> String {
    let target = offset_text(register_offset(register), "%rbp");
    match register.view {
        View::Q => format!("\tmovq {source}, {target}\n"),
        View::D => format!("\tmovl %eax, %eax\n\tmovq %rax, {target}\n"),
        View::B | View::W => {
            let (low_mask, high_mask) = if register.view == View::B {
                (0xff, -256)
            } else {
                (0xffff, -65_536)
            };
            format!(
                "\tmovq {target}, %rdx\n\tandq ${high_mask}, %rdx\n\tandq ${low_mask}, %rax\n\torq %rax, %rdx\n\tmovq %rdx, {target}\n"
            )
        }
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
        0x002a => format!(
            "\timul{suffix} {}, {}\n",
            data_register(width),
            accumulator_register(width)
        ),
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
        0x0055 => assembly.push_str(&format!("\tcall {target}\n")),
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
    let mut assembly = load_scalar(instruction, 2, width, "%rax")?;
    // Set up %rdx:%rax for division
    if opcode == 0x0005 {
        // div.u: zero-extend %rax into %rdx
        assembly.push_str("\txorq %rdx, %rdx\n");
    } else {
        // div.s: sign-extend %rax into %rdx
        assembly.push_str("\tmovq %rax, %rdx\n\tsarq $63, %rdx\n");
    }
    assembly.push_str(&load_scalar(instruction, 3, width, "%rcx")?);
    assembly.push_str(&format!(
        "\t{} %rcx\n",
        if opcode == 0x0005 { "div" } else { "idiv" }
    ));
    // Quotient in %rax, remainder in %rdx
    assembly.push_str(&write_register(dest_q, "%rax"));
    assembly.push_str(&write_register(dest_m, "%rdx"));
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
    let mut assembly = load_scalar(instruction, 2, width, "%rax")?; // expected -> %rax
    assembly.push_str(&load_scalar(instruction, 3, width, "%rbx")?); // desired -> %rbx
    // Address: [base + offset]
    let (base, _offset) = match instruction.operands.get(1) {
        Some(VerifiedOperand::Memory { base, offset }) => (*base, *offset),
        _ => return Err(error("compare.swap needs memory operand")),
    };
    assembly.push_str(&format!(
        "\tmovq {}, %rcx\n",
        offset_text(register_offset(base), "%rbp")
    ));
    assembly.push_str(&format!(
        "\t{} %rbx, (%rcx)\n",
        if width == View::Q {
            "cmpxchgq"
        } else {
            "cmpxchgl"
        }
    ));
    // Result (old value) in %rax
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
        "\tmovq $0, {}\n",
        offset_text(FLAGS_OFFSET, "%rbp")
    ));
    let entry = function.blocks.get(&0).copied().unwrap_or(0);
    assembly.push_str(&format!("\tcall .Lpc{entry}\n\tjmp .Lexit\n"));
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
            0x0003 => assembly.push_str("\tret\n"),
            0x0004 => assembly.push_str("\tjmp .Lexit\n"),
            0x0005 | 0x0006 => assembly.push_str(&emit_div(instruction)?),
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
                return Err(error(format!(
                    "opcode 0x{opcode:04x} is outside the native AOT subset"
                )));
            }
        }
    }
    assembly.push_str(&format!(
        ".Lexit:\n\tmovl {}(%rbp), %eax\n\tleave\n\tret\n.size main, .-main\n.section .note.GNU-stack,\"\",@progbits\n",
        register_offset(VerifiedRegister { index: 0, view: View::Q })
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
