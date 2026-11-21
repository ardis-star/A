use crate::model::{Predicate, View};
use crate::verifier::{
    VerifiedFunction, VerifiedInstruction, VerifiedOperand, VerifiedRegister, decode_image,
    verify_image,
};

const FLAG_ZERO: u64 = 1 << 0;
const FLAG_CARRY: u64 = 1 << 1;
const FLAG_SIGN: u64 = 1 << 2;
const FLAG_OVERFLOW: u64 = 1 << 3;
const FLAG_PARITY: u64 = 1 << 4;
const FLAG_DIRECTION: u64 = 1 << 5;
const DEFINED_FLAGS: u64 =
    FLAG_ZERO | FLAG_CARRY | FLAG_SIGN | FLAG_OVERFLOW | FLAG_PARITY | FLAG_DIRECTION;

// Capability bitmask bits
const CAP_MEMORY_MAP: u32 = 1 << 0;
const CAP_IO_PORT: u32 = 1 << 1;
const CAP_IRQ_MANAGE: u32 = 1 << 2;
const CAP_CONTEXT_SWITCH: u32 = 1 << 3;
const CAP_CPU_CONTROL: u32 = 1 << 4;
const CAP_POWER_CONTROL: u32 = 1 << 5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AvmError {
    Verification(String),
    MissingEntry,
    UnsupportedInstruction(u16),
    InvalidOperand(&'static str),
    IntegerOverflow,
    DivByZero,
    MemoryAccess { address: u64, length: usize },
    Alignment { address: u64, alignment: usize },
    Breakpoint,
    PrivilegeViolation,
    UnsupportedService,
    InvalidHandler,
    CallStackLimit,
    StepLimit,
}

impl std::fmt::Display for AvmError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Verification(message) => write!(formatter, "verification failed: {message}"),
            Self::MissingEntry => formatter.write_str("image has no entry function"),
            Self::UnsupportedInstruction(opcode) => {
                write!(formatter, "unsupported AVM opcode 0x{opcode:04x}")
            }
            Self::InvalidOperand(message) => {
                write!(formatter, "invalid verified operand: {message}")
            }
            Self::IntegerOverflow => formatter.write_str("INTEGER_OVERFLOW"),
            Self::DivByZero => formatter.write_str("DIVIDE_BY_ZERO"),
            Self::MemoryAccess { address, length } => write!(
                formatter,
                "MEMORY_ACCESS at 0x{address:016x} for {length} bytes"
            ),
            Self::Alignment { address, alignment } => write!(
                formatter,
                "ALIGNMENT at 0x{address:016x}; required alignment {alignment}"
            ),
            Self::Breakpoint => formatter.write_str("BREAKPOINT"),
            Self::PrivilegeViolation => formatter.write_str("PRIVILEGE_VIOLATION"),
            Self::UnsupportedService => formatter.write_str("UNSUPPORTED_SERVICE"),
            Self::InvalidHandler => formatter.write_str("INVALID_HANDLER"),
            Self::CallStackLimit => formatter.write_str("AVM call stack limit exceeded"),
            Self::StepLimit => formatter.write_str("AVM instruction step limit exceeded"),
        }
    }
}

impl std::error::Error for AvmError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryRegion {
    pub base: u64,
    pub bytes: Vec<u8>,
    pub readable: bool,
    pub writable: bool,
    pub shared: bool,
    pub atomic: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunResult {
    Returned,
    Stopped(u64),
}

#[derive(Debug, Clone)]
pub struct Avm {
    pub registers: [u64; 32],
    pub flags: u64,
    pub pc: usize,
    pub privileged: bool,
    pub capabilities: u32,
    pub functions: Vec<VerifiedFunction>,
    /// Exported symbols, in SYMS order; `address` operands index this list.
    pub symbols: Vec<crate::verifier::Symbol>,
    pub code_base: usize,
    current_function_id: u32,
    call_stack: Vec<(u32, usize)>,
    handler_stack: Vec<(u32, usize)>,
    regions: Vec<MemoryRegion>,
    step_limit: usize,
}

impl Avm {
    pub fn load(image: &[u8]) -> Result<Self, AvmError> {
        let decoded =
            verify_image(image).map_err(|error| AvmError::Verification(error.to_string()))?;
        let entry = decoded.entry_function.ok_or(AvmError::MissingEntry)?;
        let entry_function = decoded
            .functions
            .iter()
            .find(|function| function.id == entry)
            .ok_or(AvmError::MissingEntry)?;
        let code_base = entry_function.code_offset;
        Ok(Self {
            registers: [0; 32],
            flags: 0,
            pc: 0,
            privileged: false,
            capabilities: 0,
            functions: decoded.functions.clone(),
            symbols: decoded.symbols.clone(),
            code_base,
            current_function_id: entry,
            call_stack: Vec::new(),
            handler_stack: Vec::new(),
            regions: Vec::new(),
            step_limit: 10_000_000,
        })
    }

    pub fn load_unverified(image: &[u8]) -> Result<Self, AvmError> {
        let decoded =
            decode_image(image).map_err(|error| AvmError::Verification(error.to_string()))?;
        let entry = decoded.entry_function.ok_or(AvmError::MissingEntry)?;
        let entry_function = decoded
            .functions
            .iter()
            .find(|function| function.id == entry)
            .ok_or(AvmError::MissingEntry)?;
        let code_base = entry_function.code_offset;
        Ok(Self {
            registers: [0; 32],
            flags: 0,
            pc: 0,
            privileged: false,
            capabilities: 0,
            functions: decoded.functions.clone(),
            symbols: decoded.symbols.clone(),
            code_base,
            current_function_id: entry,
            call_stack: Vec::new(),
            handler_stack: Vec::new(),
            regions: Vec::new(),
            step_limit: 10_000_000,
        })
    }

    pub fn with_step_limit(mut self, step_limit: usize) -> Self {
        self.step_limit = step_limit;
        self
    }

    pub fn set_privileged(mut self, privileged: bool) -> Self {
        self.privileged = privileged;
        self
    }

    pub fn with_capabilities(mut self, capabilities: u32) -> Self {
        self.capabilities = capabilities;
        self
    }

    pub fn map_region(&mut self, region: MemoryRegion) -> Result<(), AvmError> {
        let end =
            region
                .base
                .checked_add(region.bytes.len() as u64)
                .ok_or(AvmError::MemoryAccess {
                    address: region.base,
                    length: region.bytes.len(),
                })?;
        if self.regions.iter().any(|existing| {
            let existing_end = existing.base.saturating_add(existing.bytes.len() as u64);
            region.base < existing_end && existing.base < end
        }) {
            return Err(AvmError::MemoryAccess {
                address: region.base,
                length: region.bytes.len(),
            });
        }
        self.regions.push(region);
        self.regions.sort_by_key(|item| item.base);
        Ok(())
    }

    pub fn memory_region(&self, base: u64) -> Option<&MemoryRegion> {
        self.regions.iter().find(|region| region.base == base)
    }

    fn current_function(&self) -> Option<&VerifiedFunction> {
        self.functions
            .iter()
            .find(|f| f.id == self.current_function_id)
    }

    /// Execute until the program returns, stops, or faults.
    ///
    /// The step budget is consumed by every instruction, including one that
    /// branches or calls. When the budget runs out the machine is left in a
    /// restartable state: the [program counter](Self::pc) points at the next
    /// instruction that would have executed (inside the function the last
    /// instruction transferred to), so a caller may resume with another `run`.
    pub fn run(&mut self) -> Result<RunResult, AvmError> {
        for _ in 0..self.step_limit {
            let function = self.current_function().ok_or(AvmError::MissingEntry)?;
            if self.pc >= function.instructions.len() {
                return Ok(RunResult::Returned);
            }
            let instruction_pc = self.pc;
            let instruction = function.instructions[instruction_pc].clone();
            self.pc = instruction_pc + 1;
            match self.execute(&instruction) {
                Ok(Some(result)) => return Ok(result),
                Ok(None) => {}
                Err(error) => {
                    // A fault must not commit its effects: point the machine
                    // back at the faulting instruction.
                    self.pc = instruction_pc;
                    return Err(error);
                }
            }
        }
        Err(AvmError::StepLimit)
    }

    /// Execute one instruction.
    ///
    /// The program counter has already been advanced past the instruction; an
    /// instruction that transfers control overwrites it with its target. The
    /// caller restores the counter if execution faults.
    fn execute(
        &mut self,
        instruction: &VerifiedInstruction,
    ) -> Result<Option<RunResult>, AvmError> {
        match instruction.opcode {
            0x0001 => Ok(None),
            0x0002 => Err(AvmError::Breakpoint),
            0x0003 => {
                if let Some((_, return_pc)) = self.call_stack.pop() {
                    self.pc = return_pc;
                    Ok(None)
                } else {
                    Ok(Some(RunResult::Returned))
                }
            }
            0x0004 => Ok(Some(RunResult::Stopped(self.registers[0]))),
            0x0005 => self.execute_div(instruction, false).map(|_| None),
            0x0006 => self.execute_div(instruction, true).map(|_| None),
            0x0007 => {
                let target = self.block_target(instruction)?;
                self.handler_stack.push((self.current_function_id, target));
                Ok(None)
            }
            0x0008 => {
                self.handler_stack.pop().ok_or(AvmError::InvalidHandler)?;
                Ok(None)
            }
            0x0009 => {
                let code = match instruction.operands.first() {
                    Some(VerifiedOperand::Unsigned(value)) => *value,
                    Some(VerifiedOperand::Signed(value)) => *value as u64,
                    _ => self.registers[0],
                };
                self.registers[0] = code;
                self.pc = self.take_handler()?;
                Ok(None)
            }
            0x000A => {
                let value = match instruction.operands.first() {
                    Some(VerifiedOperand::Unsigned(value)) => *value != 0,
                    Some(VerifiedOperand::Signed(value)) => *value != 0,
                    _ => return Err(AvmError::InvalidOperand("flag.direction requires a mode")),
                };
                if value {
                    self.flags |= FLAG_DIRECTION;
                } else {
                    self.flags &= !FLAG_DIRECTION;
                }
                self.flags &= DEFINED_FLAGS;
                Ok(None)
            }
            0x000B => {
                let destination = operand_register(instruction, 0)?;
                let symbol_index = match instruction.operands.get(1) {
                    Some(VerifiedOperand::Symbol(index)) => *index,
                    _ => return Err(AvmError::InvalidOperand("address requires a symbol")),
                };
                let address = self.resolve_symbol_address(symbol_index)?;
                self.write_register(destination, address as u64);
                Ok(None)
            }
            0x000C => {
                // fence - no-op on sequential execution; ordering is architectural
                Ok(None)
            }
            0x000D => self.execute_compare_swap(instruction).map(|_| None),
            0x0010 => {
                let width = instruction
                    .width
                    .ok_or(AvmError::InvalidOperand("move width"))?;
                let destination = operand_register(instruction, 0)?;
                let value = self.read_scalar(instruction, 1, width)?;
                self.write_register(destination, value);
                Ok(None)
            }
            0x0011 | 0x0012 => {
                let source_width = instruction
                    .width
                    .ok_or(AvmError::InvalidOperand("extension source width"))?;
                let destination = operand_register(instruction, 0)?;
                let source = self.read_scalar(instruction, 1, source_width)?;
                let value = if instruction.opcode == 0x0011 {
                    signed_value(source, source_width.bits()) as u64
                } else {
                    source
                };
                self.write_register(destination, value);
                Ok(None)
            }
            0x0013 => {
                let destination = operand_register(instruction, 0)?;
                let Some(VerifiedOperand::Predicate(predicate)) = instruction.operands.get(1)
                else {
                    return Err(AvmError::InvalidOperand("expected flag predicate"));
                };
                let width = instruction
                    .width
                    .ok_or(AvmError::InvalidOperand("select width"))?;
                let source_index = if self.predicate(*predicate) { 2 } else { 3 };
                let value = self.read_scalar(instruction, source_index, width)?;
                self.write_register(destination, value);
                Ok(None)
            }
            0x0020..=0x002f | 0x0030..=0x003f | 0x0048..=0x004e => {
                self.execute_alu(instruction)?;
                Ok(None)
            }
            0x0040..=0x0043 => {
                let width = instruction
                    .width
                    .ok_or(AvmError::InvalidOperand("load width"))?;
                let destination = operand_register(instruction, 0)?;
                let address = self.memory_address(instruction, 1)?;
                let value = self.load_scalar(address, width, false)?;
                self.write_register(destination, value);
                Ok(None)
            }
            0x0044..=0x0047 => {
                let width = instruction
                    .width
                    .ok_or(AvmError::InvalidOperand("store width"))?;
                let (base, offset) = operand_memory(instruction, 0)?;
                let address = self.register_value(base).checked_add_signed(offset).ok_or(
                    AvmError::MemoryAccess {
                        address: self.register_value(base),
                        length: width_bytes(width),
                    },
                )?;
                let value = self.read_scalar(instruction, 1, width)?;
                self.store_scalar(address, width, value, false)?;
                Ok(None)
            }
            0x0080..=0x0083 => {
                let width = instruction
                    .width
                    .ok_or(AvmError::InvalidOperand("acquire load width"))?;
                let destination = operand_register(instruction, 0)?;
                let address = self.memory_address(instruction, 1)?;
                let value = self.load_scalar(address, width, true)?;
                self.write_register(destination, value);
                Ok(None)
            }
            0x0084..=0x0087 => {
                let width = instruction
                    .width
                    .ok_or(AvmError::InvalidOperand("release store width"))?;
                let (base, offset) = operand_memory(instruction, 0)?;
                let address = self.register_value(base).checked_add_signed(offset).ok_or(
                    AvmError::MemoryAccess {
                        address: self.register_value(base),
                        length: width_bytes(width),
                    },
                )?;
                let value = self.read_scalar(instruction, 1, width)?;
                self.store_scalar(address, width, value, true)?;
                Ok(None)
            }
            0x0060 => {
                let left = operand_register(instruction, 0)?;
                let right = operand_register(instruction, 1)?;
                self.registers
                    .swap(usize::from(left.index), usize::from(right.index));
                Ok(None)
            }
            0x0050..=0x0056 | 0x0061..=0x0066 => {
                let target = self.block_target(instruction)?;
                let take_branch = match instruction.opcode {
                    0x0050 | 0x0055 | 0x0056 => true,
                    0x0051 => self.flags & FLAG_ZERO != 0,
                    0x0052 => self.flags & FLAG_ZERO == 0,
                    0x0053 => self.flags & FLAG_CARRY != 0,
                    0x0054 => self.flags & FLAG_CARRY == 0,
                    0x0061 => self.flags & FLAG_SIGN != 0,
                    0x0062 => self.flags & FLAG_SIGN == 0,
                    0x0063 => self.flags & FLAG_OVERFLOW != 0,
                    0x0064 => self.flags & FLAG_OVERFLOW == 0,
                    0x0065 => self.flags & FLAG_PARITY != 0,
                    0x0066 => self.flags & FLAG_PARITY == 0,
                    _ => return Err(AvmError::UnsupportedInstruction(instruction.opcode)),
                };
                if take_branch {
                    if instruction.opcode == 0x0055 {
                        if self.call_stack.len() >= 1_048_576 {
                            return Err(AvmError::CallStackLimit);
                        }
                        self.call_stack.push((self.current_function_id, self.pc));
                    }
                    self.pc = target;
                }
                Ok(None)
            }
            0x0057..=0x005F | 0x0067 | 0x0068 => self.execute_sys(instruction).map(|_| None),
            opcode => Err(AvmError::UnsupportedInstruction(opcode)),
        }
    }

    fn execute_div(
        &mut self,
        instruction: &VerifiedInstruction,
        signed: bool,
    ) -> Result<(), AvmError> {
        let width = instruction
            .width
            .ok_or(AvmError::InvalidOperand("div width"))?;
        let bits = width.bits();
        let mask = width_mask(width);
        let dest_q = operand_register(instruction, 0)?;
        let dest_m = operand_register(instruction, 1)?;
        let left = self.read_scalar(instruction, 2, width)? & mask;
        let right = self.read_scalar(instruction, 3, width)? & mask;

        if right == 0 {
            return Err(AvmError::DivByZero);
        }

        let (quotient, remainder) = if signed {
            let l = signed_value(left, bits);
            let r = signed_value(right, bits);
            let min = -(1_i128 << (bits - 1));
            // The only quotient that is not representable at this width; the
            // architecture reports it as an integer overflow, not a divide fault.
            if l == min && r == -1 {
                return Err(AvmError::IntegerOverflow);
            }
            let q = (l / r) as u64 & mask;
            let m = (l % r) as u64 & mask;
            (q, m)
        } else {
            let q = left.wrapping_div(right) & mask;
            let m = left.wrapping_rem(right) & mask;
            (q, m)
        };

        self.write_register(dest_q, quotient);
        self.write_register(dest_m, remainder);
        Ok(())
    }

    fn execute_compare_swap(&mut self, instruction: &VerifiedInstruction) -> Result<(), AvmError> {
        let width = instruction
            .width
            .ok_or(AvmError::InvalidOperand("compare.swap width"))?;
        let mask = width_mask(width);
        let destination = operand_register(instruction, 0)?;
        let (base, offset) = operand_memory(instruction, 1)?;
        let address =
            self.register_value(base)
                .checked_add_signed(offset)
                .ok_or(AvmError::MemoryAccess {
                    address: self.register_value(base),
                    length: width_bytes(width),
                })?;
        let expected = self.read_scalar(instruction, 2, width)? & mask;
        let desired = self.read_scalar(instruction, 3, width)? & mask;

        // Atomic requires natural alignment and shared+atomic region
        let length = width_bytes(width);
        if !address.is_multiple_of(length as u64) {
            return Err(AvmError::Alignment {
                address,
                alignment: length,
            });
        }
        let (index, range) = self.region_index_range(address, length, true, true)?;
        let mut bytes = [0u8; 8];
        bytes[..length].copy_from_slice(&self.regions[index].bytes[range.start..range.end]);
        let old = u64::from_le_bytes(bytes);
        if old == expected {
            let bytes = desired.to_le_bytes();
            self.regions[index].bytes[range.start..range.start + length]
                .copy_from_slice(&bytes[..length]);
        }
        self.write_register(destination, old);
        Ok(())
    }

    fn execute_sys(&mut self, instruction: &VerifiedInstruction) -> Result<(), AvmError> {
        if !self.privileged {
            return Err(AvmError::PrivilegeViolation);
        }
        let service = match instruction.opcode {
            0x0057 => CAP_MEMORY_MAP,
            0x0058 => CAP_MEMORY_MAP,
            0x0059 => CAP_IO_PORT,
            0x005A => CAP_IO_PORT,
            0x005B => CAP_IRQ_MANAGE,
            0x005C => CAP_IRQ_MANAGE,
            0x005D => CAP_IRQ_MANAGE,
            0x005E => CAP_CONTEXT_SWITCH,
            0x005F => CAP_CONTEXT_SWITCH,
            0x0067 => CAP_CPU_CONTROL,
            0x0068 => CAP_POWER_CONTROL,
            _ => return Err(AvmError::UnsupportedInstruction(instruction.opcode)),
        };
        if self.capabilities & service == 0 {
            return Err(AvmError::PrivilegeViolation);
        }
        Err(AvmError::UnsupportedService)
    }

    /// Pop the innermost handler recorded *by the running function* and return
    /// its instruction index.
    ///
    /// Handlers are function-scoped: a `catch` in one function must never
    /// redirect a `raise` that happens in another, because block IDs are only
    /// meaningful inside the function that defines them. Handlers registered by
    /// an outer function are discarded by the unwind that the `raise` performs,
    /// so they are popped here rather than left to redirect a later `raise` to a
    /// stale block in the wrong function.
    fn take_handler(&mut self) -> Result<usize, AvmError> {
        let function = self.current_function_id;
        let mut found = None;
        while let Some((owner, target)) = self.handler_stack.pop() {
            if owner == function {
                found = Some(target);
                break;
            }
            // A handler registered by a callee that has already returned can no
            // longer be reached; discard it and keep unwinding.
            if self
                .call_stack
                .iter()
                .any(|(frame_function, _)| *frame_function == owner)
            {
                self.handler_stack.push((owner, target));
                break;
            }
        }
        found.ok_or(AvmError::InvalidHandler)
    }

    fn block_target(&self, instruction: &VerifiedInstruction) -> Result<usize, AvmError> {
        let Some(VerifiedOperand::Block(block_id)) = instruction.operands.first() else {
            return Err(AvmError::InvalidOperand("expected block target"));
        };
        let function = self.current_function().ok_or(AvmError::MissingEntry)?;
        function
            .blocks
            .get(block_id)
            .copied()
            .ok_or(AvmError::InvalidOperand("unmapped block target"))
    }

    /// Resolve a symbol operand to a VM address.
    ///
    /// `Symbol(n)` indexes the image's SYMS section, not the function table, so
    /// the name is resolved through that section to the function it names and
    /// then to that function's address in the VM's code space. Programs must
    /// therefore only rely on the *identity* of the returned address; it is not
    /// a host pointer and has no meaning outside the VM.
    fn resolve_symbol_address(&self, symbol_index: u32) -> Result<usize, AvmError> {
        let symbol = self
            .symbols
            .get(symbol_index as usize)
            .ok_or(AvmError::InvalidOperand(
                "symbol index outside SYMS section",
            ))?;
        let function = self
            .functions
            .iter()
            .find(|function| function.id == symbol.function)
            .ok_or(AvmError::InvalidOperand(
                "symbol names a function outside the table",
            ))?;
        Ok(self.code_base + function.code_offset)
    }

    fn predicate(&self, predicate: Predicate) -> bool {
        match predicate {
            Predicate::Zero => self.flags & FLAG_ZERO != 0,
            Predicate::NotZero => self.flags & FLAG_ZERO == 0,
            Predicate::Carry => self.flags & FLAG_CARRY != 0,
            Predicate::NoCarry => self.flags & FLAG_CARRY == 0,
            Predicate::Sign => self.flags & FLAG_SIGN != 0,
            Predicate::NotSign => self.flags & FLAG_SIGN == 0,
            Predicate::Overflow => self.flags & FLAG_OVERFLOW != 0,
            Predicate::NoOverflow => self.flags & FLAG_OVERFLOW == 0,
            Predicate::Even => self.flags & FLAG_PARITY != 0,
            Predicate::Odd => self.flags & FLAG_PARITY == 0,
        }
    }

    fn execute_alu(&mut self, instruction: &VerifiedInstruction) -> Result<(), AvmError> {
        let width = instruction
            .width
            .ok_or(AvmError::InvalidOperand("ALU width"))?;
        let bits = width.bits();
        let mask = width_mask(width);
        let is_test = matches!(instruction.opcode, 0x0024 | 0x004e);
        let unary = matches!(instruction.opcode, 0x002c..=0x002f | 0x0036..=0x0039);
        let destination = if is_test {
            None
        } else {
            Some(operand_register(instruction, 0)?)
        };
        let left_index = if destination.is_some() { 1 } else { 0 };
        let right_index = if unary {
            None
        } else if destination.is_some() {
            Some(2)
        } else {
            Some(1)
        };
        let left = self.read_scalar(instruction, left_index, width)? & mask;
        let right = right_index
            .map(|index| self.read_scalar(instruction, index, width))
            .transpose()?
            .unwrap_or(0);
        let right = right & mask;
        let carry_in = u64::from(self.flags & FLAG_CARRY != 0);
        let (result, carry, overflow) = match instruction.opcode {
            0x0020 | 0x0021 | 0x002b => {
                let wide = u128::from(left) + u128::from(right);
                let result = (wide as u64) & mask;
                if instruction.opcode == 0x002b {
                    let sum = signed_value(left, bits) + signed_value(right, bits);
                    if !signed_fits(sum, bits) {
                        return Err(AvmError::IntegerOverflow);
                    }
                }
                (
                    result,
                    wide > u128::from(mask),
                    signed_add_overflow(left, right, result, bits),
                )
            }
            0x0025 | 0x0026 => {
                let wide = u128::from(left) + u128::from(right) + u128::from(carry_in);
                let result = (wide as u64) & mask;
                let signed_sum =
                    signed_value(left, bits) + signed_value(right, bits) + i128::from(carry_in);
                (
                    result,
                    wide > u128::from(mask),
                    !signed_fits(signed_sum, bits),
                )
            }
            0x0022..=0x0024 => {
                let result = left.wrapping_sub(right) & mask;
                (
                    result,
                    left < right,
                    signed_sub_overflow(left, right, result, bits),
                )
            }
            0x0027 | 0x0028 => {
                let rhs = u128::from(right) + u128::from(carry_in);
                let result = left.wrapping_sub(rhs as u64) & mask;
                let signed_difference =
                    signed_value(left, bits) - signed_value(right, bits) - i128::from(carry_in);
                (
                    result,
                    u128::from(left) < rhs,
                    !signed_fits(signed_difference, bits),
                )
            }
            0x0029 | 0x002a => {
                let result = left.wrapping_mul(right) & mask;
                (result, false, false)
            }
            0x002c | 0x002d => {
                let result = 0_u64.wrapping_sub(left) & mask;
                (result, left != 0, left == (1_u64 << (bits - 1)))
            }
            0x002e | 0x002f => {
                let result = left.wrapping_add(1) & mask;
                (result, left == mask, left == (1_u64 << (bits - 1)) - 1)
            }
            0x0036 | 0x0037 => {
                let result = left.wrapping_sub(1) & mask;
                (result, left == 0, left == (1_u64 << (bits - 1)))
            }
            0x0030 | 0x0031 => (left & right, false, false),
            0x0032 | 0x0033 => (left | right, false, false),
            0x0034 | 0x0035 => (left ^ right, false, false),
            0x0038 | 0x0039 => (!left & mask, false, false),
            0x003a..=0x003f | 0x0048..=0x004b => {
                shift_result(instruction.opcode, left, right, bits, mask)
            }
            0x004c | 0x004d => {
                let bit = (right % u64::from(bits)) as u32;
                let value = if instruction.opcode == 0x004c {
                    left | (1_u64 << bit)
                } else {
                    left & !(1_u64 << bit)
                };
                (value & mask, false, false)
            }
            0x004e => {
                let bit = (right % u64::from(bits)) as u32;
                (u64::from(left & (1_u64 << bit) != 0), false, false)
            }
            _ => return Err(AvmError::UnsupportedInstruction(instruction.opcode)),
        };
        if let Some(destination) = destination {
            self.write_register(destination, result);
        }
        if updates_flags(instruction.opcode) {
            self.update_result_flags(result, bits, mask, carry, overflow);
        }
        Ok(())
    }

    fn update_result_flags(
        &mut self,
        result: u64,
        bits: u32,
        mask: u64,
        carry: bool,
        overflow: bool,
    ) {
        let mut flags = self.flags & FLAG_DIRECTION;
        if result & mask == 0 {
            flags |= FLAG_ZERO;
        }
        if result & (1_u64 << (bits - 1)) != 0 {
            flags |= FLAG_SIGN;
        }
        if (result as u8).count_ones().is_multiple_of(2) {
            flags |= FLAG_PARITY;
        }
        if carry {
            flags |= FLAG_CARRY;
        }
        if overflow {
            flags |= FLAG_OVERFLOW;
        }
        self.flags = flags & DEFINED_FLAGS;
    }

    fn read_scalar(
        &self,
        instruction: &VerifiedInstruction,
        index: usize,
        operation_width: View,
    ) -> Result<u64, AvmError> {
        match instruction
            .operands
            .get(index)
            .ok_or(AvmError::InvalidOperand("missing scalar"))?
        {
            VerifiedOperand::Register(register) => {
                Ok(self.register_value(*register) & width_mask(operation_width))
            }
            VerifiedOperand::Unsigned(value) => Ok(*value & width_mask(operation_width)),
            VerifiedOperand::Signed(value) => Ok(*value as u64 & width_mask(operation_width)),
            _ => Err(AvmError::InvalidOperand("expected scalar value")),
        }
    }

    fn register_value(&self, register: VerifiedRegister) -> u64 {
        self.registers[usize::from(register.index)] & width_mask(register.view)
    }

    fn write_register(&mut self, register: VerifiedRegister, value: u64) {
        let slot = &mut self.registers[usize::from(register.index)];
        match register.view {
            View::B => *slot = (*slot & !0xff) | (value & 0xff),
            View::W => *slot = (*slot & !0xffff) | (value & 0xffff),
            View::D => *slot = value & 0xffff_ffff,
            View::Q => *slot = value,
        }
    }

    fn memory_address(
        &self,
        instruction: &VerifiedInstruction,
        index: usize,
    ) -> Result<u64, AvmError> {
        let (base, offset) = operand_memory(instruction, index)?;
        self.register_value(base)
            .checked_add_signed(offset)
            .ok_or(AvmError::MemoryAccess {
                address: self.register_value(base),
                length: instruction.width.map(width_bytes).unwrap_or(0),
            })
    }

    fn load_scalar(&self, address: u64, width: View, atomic: bool) -> Result<u64, AvmError> {
        let length = width_bytes(width);
        let (index, range) = self.region_index_range(address, length, false, atomic)?;
        let mut bytes = [0; 8];
        bytes[..length].copy_from_slice(&self.regions[index].bytes[range]);
        Ok(u64::from_le_bytes(bytes))
    }

    fn store_scalar(
        &mut self,
        address: u64,
        width: View,
        value: u64,
        atomic: bool,
    ) -> Result<(), AvmError> {
        let length = width_bytes(width);
        let range = self.region_index_range(address, length, true, atomic)?;
        let bytes = value.to_le_bytes();
        self.regions[range.0].bytes[range.1].copy_from_slice(&bytes[..length]);
        Ok(())
    }

    fn region_index_range(
        &self,
        address: u64,
        length: usize,
        write: bool,
        atomic: bool,
    ) -> Result<(usize, std::ops::Range<usize>), AvmError> {
        if atomic && !address.is_multiple_of(length as u64) {
            return Err(AvmError::Alignment {
                address,
                alignment: length,
            });
        }
        let end = address
            .checked_add(length as u64)
            .ok_or(AvmError::MemoryAccess { address, length })?;
        for (index, region) in self.regions.iter().enumerate() {
            let Some(region_end) = region.base.checked_add(region.bytes.len() as u64) else {
                continue;
            };
            if address >= region.base
                && end <= region_end
                && (if write {
                    region.writable
                } else {
                    region.readable
                })
                && (!atomic || (region.shared && region.atomic))
            {
                let start = usize::try_from(address - region.base)
                    .map_err(|_| AvmError::MemoryAccess { address, length })?;
                return Ok((index, start..start + length));
            }
        }
        Err(AvmError::MemoryAccess { address, length })
    }
}

fn operand_register(
    instruction: &VerifiedInstruction,
    index: usize,
) -> Result<VerifiedRegister, AvmError> {
    match instruction.operands.get(index) {
        Some(VerifiedOperand::Register(register)) => Ok(*register),
        _ => Err(AvmError::InvalidOperand("expected register")),
    }
}

fn operand_memory(
    instruction: &VerifiedInstruction,
    index: usize,
) -> Result<(VerifiedRegister, i64), AvmError> {
    match instruction.operands.get(index) {
        Some(VerifiedOperand::Memory { base, offset }) => Ok((*base, *offset)),
        _ => Err(AvmError::InvalidOperand("expected memory expression")),
    }
}

fn width_mask(width: View) -> u64 {
    match width {
        View::B => 0xff,
        View::W => 0xffff,
        View::D => 0xffff_ffff,
        View::Q => u64::MAX,
    }
}

fn width_bytes(width: View) -> usize {
    (width.bits() / 8) as usize
}

fn signed_add_overflow(left: u64, right: u64, result: u64, bits: u32) -> bool {
    let sign = 1_u64 << (bits - 1);
    (!(left ^ right) & (left ^ result) & sign) != 0
}

fn signed_sub_overflow(left: u64, right: u64, result: u64, bits: u32) -> bool {
    let sign = 1_u64 << (bits - 1);
    ((left ^ right) & (left ^ result) & sign) != 0
}

fn updates_flags(opcode: u16) -> bool {
    matches!(
        opcode,
        0x0021
            | 0x0023
            | 0x0024
            | 0x0026
            | 0x0028
            | 0x002a
            | 0x002d
            | 0x002f
            | 0x0031
            | 0x0033
            | 0x0035
            | 0x0037
            | 0x0039
            | 0x003b
            | 0x003d
            | 0x003f
            | 0x0049
            | 0x004b
            | 0x004e
    )
}

fn signed_value(value: u64, bits: u32) -> i128 {
    let value = i128::from(value & width_mask_for_bits(bits));
    let sign_bit = 1_i128 << (bits - 1);
    if value & sign_bit == 0 {
        value
    } else {
        value - (1_i128 << bits)
    }
}

fn signed_fits(value: i128, bits: u32) -> bool {
    let minimum = -(1_i128 << (bits - 1));
    let maximum = (1_i128 << (bits - 1)) - 1;
    (minimum..=maximum).contains(&value)
}

fn width_mask_for_bits(bits: u32) -> u64 {
    if bits == 64 {
        u64::MAX
    } else {
        (1_u64 << bits) - 1
    }
}

fn shift_result(
    opcode: u16,
    value: u64,
    raw_count: u64,
    bits: u32,
    mask: u64,
) -> (u64, bool, bool) {
    let count = (raw_count % u64::from(bits)) as u32;
    let sign = 1_u64 << (bits - 1);
    if count == 0 {
        return (value & mask, false, false);
    }
    match opcode {
        0x003a | 0x003b => {
            let result = value.wrapping_shl(count) & mask;
            let carry = value & (1_u64 << (bits - count)) != 0;
            let overflow = opcode == 0x003b && count == 1 && ((value ^ result) & sign != 0);
            (result, carry, overflow)
        }
        0x003c | 0x003d => (value >> count, value & (1_u64 << (count - 1)) != 0, false),
        0x003e | 0x003f => (
            (signed_value(value, bits) >> count) as u64 & mask,
            value & (1_u64 << (count - 1)) != 0,
            false,
        ),
        0x0048 | 0x0049 => {
            let result = (value.wrapping_shl(count) | (value >> (bits - count))) & mask;
            (result, value & (1_u64 << (bits - count)) != 0, false)
        }
        0x004a | 0x004b => {
            let result = ((value >> count) | value.wrapping_shl(bits - count)) & mask;
            (result, value & (1_u64 << (count - 1)) != 0, false)
        }
        _ => (0, false, false),
    }
}
