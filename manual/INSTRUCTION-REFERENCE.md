# +A Instruction Reference

**Edition:** ASCENSION +A Core V1.1.0

Complete list of the 97 instructions the V1.1.0 toolchain defines. The
table is generated from `rust/src/instructions.rs` and `rust/src/native.rs`,
and `instruction_reference_covers_every_defined_mnemonic` in
`rust/tests/regressions.rs` fails if this file disagrees with the code in
either direction, so it cannot drift.

Column meanings:

* **Opcode** — the 16-bit opcode in the bytecode image.
* **Form** — the operand syntax accepted by the parser.
* **Width?** — `yes` when the instruction needs a register operand to fix
  its operation width.
* **Native?** — `yes` when the JIT and AOT backends can lower it. Anything
  marked `—` is interpreter-only and is reported as an error, never
  silently interpreted, when a native run is requested.
  See [JIT-AOT.md](JIT-AOT.md).

## Control and lifecycle

| Mnemonic | Opcode | Form | Width? | Native? | Notes |
|---|---|---|---|---|---|
| `nop` | `0x0001` | `mnemonic` | — | yes | No operation. |
| `break` | `0x0002` | `mnemonic` | — | — | Raise `BREAKPOINT`. |
| `return` | `0x0003` | `mnemonic` | — | yes | Pop a return point; with an empty stack, end the program with `returned`. |
| `stop` | `0x0004` | `mnemonic` | — | yes | End the program with `stopped: R0`. |

## Branches

| Mnemonic | Opcode | Form | Width? | Native? | Notes |
|---|---|---|---|---|---|
| `jump` | `0x0050` | `mnemonic -> label` | — | yes | Unconditional transfer. |
| `if.zero` | `0x0051` | `mnemonic -> label` | — | yes |  |
| `if.notzero` | `0x0052` | `mnemonic -> label` | — | yes |  |
| `if.carry` | `0x0053` | `mnemonic -> label` | — | yes |  |
| `if.nocarry` | `0x0054` | `mnemonic -> label` | — | yes |  |
| `if.sign` | `0x0061` | `mnemonic -> label` | — | yes |  |
| `if.notsign` | `0x0062` | `mnemonic -> label` | — | yes |  |
| `if.overflow` | `0x0063` | `mnemonic -> label` | — | yes |  |
| `if.nooverflow` | `0x0064` | `mnemonic -> label` | — | yes |  |
| `if.even` | `0x0065` | `mnemonic -> label` | — | yes |  |
| `if.odd` | `0x0066` | `mnemonic -> label` | — | yes |  |

## Calls

| Mnemonic | Opcode | Form | Width? | Native? | Notes |
|---|---|---|---|---|---|
| `call` | `0x0055` | `mnemonic -> label` | — | yes | Push the return point, then transfer. |
| `tailcall` | `0x0056` | `mnemonic -> label` | — | yes | Transfer without pushing a return point. |

## Exceptions and ordering

| Mnemonic | Opcode | Form | Width? | Native? | Notes |
|---|---|---|---|---|---|
| `catch` | `0x0007` | `mnemonic -> label` | — | — | Push `handler` as the current function's innermost handler. |
| `uncatch` | `0x0008` | `mnemonic` | — | — | Pop the current function's innermost handler. |
| `raise` | `0x0009` | `mnemonic mode` | — | — | Transfer to the current function's innermost handler, putting the code in `R0`. |
| `flag.direction` | `0x000A` | `mnemonic mode` | — | yes | `forward` clears D, `backward` sets D. Other flags are preserved. |
| `fence` | `0x000C` | `mnemonic mode` | — | yes | Memory ordering: `acquire`, `release`, or `full`. A no-op under sequential execution. |

## Data movement

| Mnemonic | Opcode | Form | Width? | Native? | Notes |
|---|---|---|---|---|---|
| `move` | `0x0010` | `mnemonic dest <- src` | yes | yes | Copy a scalar. |
| `extend.s` | `0x0011` | `mnemonic dest <- src` | yes | — | Sign-extend a narrower source into a wider destination. |
| `extend.z` | `0x0012` | `mnemonic dest <- src` | yes | — | Zero-extend a narrower source into a wider destination. |
| `select` | `0x0013` | `mnemonic dest <- pred, a, b` | yes | — | Copy `a` when `pred` holds, otherwise `b`. Flags are untouched. |

## Addition and subtraction

| Mnemonic | Opcode | Form | Width? | Native? | Notes |
|---|---|---|---|---|---|
| `sum` | `0x0020` | `mnemonic dest <- a, b` | yes | yes | Add without updating flags. |
| `sum!` | `0x0021` | `mnemonic dest <- a, b` | yes | yes | Add and update flags (`carry` unsigned overflow, `overflow` signed). |
| `diff` | `0x0022` | `mnemonic dest <- a, b` | yes | yes | Subtract without updating flags. |
| `diff!` | `0x0023` | `mnemonic dest <- a, b` | yes | yes | Subtract and update flags. |
| `compare!` | `0x0024` | `mnemonic a, b` | yes | yes | Subtract for flags only; no destination is written. |
| `sumc` | `0x0025` | `mnemonic dest <- a, b` | yes | — | Add with carry-in, without updating flags; C is an input. |
| `sumc!` | `0x0026` | `mnemonic dest <- a, b` | yes | — | Add with carry-in and update flags. |
| `diffc` | `0x0027` | `mnemonic dest <- a, b` | yes | — | Subtract with borrow, without updating flags; C is an input. |
| `diffc!` | `0x0028` | `mnemonic dest <- a, b` | yes | — | Subtract with borrow and update flags. |
| `checked.sum` | `0x002B` | `mnemonic dest <- a, b` | yes | — | Add, faulting with `INTEGER_OVERFLOW` instead of wrapping. The destination is not committed on fault. |
| `neg` | `0x002C` | `mnemonic dest <- src` | yes | — | Two's-complement negate. |
| `neg!` | `0x002D` | `mnemonic dest <- src` | yes | — | Negate and update flags. |
| `inc` | `0x002E` | `mnemonic dest <- src` | yes | — | Increment. |
| `inc!` | `0x002F` | `mnemonic dest <- src` | yes | — | Increment and update flags. |
| `dec` | `0x0036` | `mnemonic dest <- src` | yes | — | Decrement. |
| `dec!` | `0x0037` | `mnemonic dest <- src` | yes | — | Decrement and update flags. |

## Multiplication and division

| Mnemonic | Opcode | Form | Width? | Native? | Notes |
|---|---|---|---|---|---|
| `div.u` | `0x0005` | `mnemonic dq, dm <- a, b` | yes | yes | Unsigned divide: quotient to the first destination, remainder to the second. Flags untouched. |
| `div.s` | `0x0006` | `mnemonic dq, dm <- a, b` | yes | yes | Signed divide, rounding toward zero. Flags untouched. |
| `mul` | `0x0029` | `mnemonic dest <- a, b` | yes | — | Truncating multiply; flags untouched. |
| `mul!` | `0x002A` | `mnemonic dest <- a, b` | yes | yes | Truncating multiply; clears C and O, updates Z/S/P. |

## Bitwise

| Mnemonic | Opcode | Form | Width? | Native? | Notes |
|---|---|---|---|---|---|
| `and` | `0x0030` | `mnemonic dest <- a, b` | yes | yes | Bitwise AND, flags untouched. |
| `and!` | `0x0031` | `mnemonic dest <- a, b` | yes | yes | Bitwise AND, then Z/S/P from the result; C and O cleared. |
| `or` | `0x0032` | `mnemonic dest <- a, b` | yes | yes | Bitwise OR, flags untouched. |
| `or!` | `0x0033` | `mnemonic dest <- a, b` | yes | yes | Bitwise OR, then Z/S/P from the result; C and O cleared. |
| `xor` | `0x0034` | `mnemonic dest <- a, b` | yes | yes | Bitwise XOR, flags untouched. |
| `xor!` | `0x0035` | `mnemonic dest <- a, b` | yes | yes | Bitwise XOR, then Z/S/P from the result; C and O cleared. |
| `not` | `0x0038` | `mnemonic dest <- src` | yes | — | Bitwise NOT, flags untouched. |
| `not!` | `0x0039` | `mnemonic dest <- src` | yes | — | Bitwise NOT, then Z/S/P from the result; C and O cleared. |

## Shifts and rotates

| Mnemonic | Opcode | Form | Width? | Native? | Notes |
|---|---|---|---|---|---|
| `shift.l` | `0x003A` | `mnemonic dest <- a, b` | yes | — | Logical left shift by `count mod width`; C receives the last bit shifted out. |
| `shift.l!` | `0x003B` | `mnemonic dest <- a, b` | yes | — | Logical left shift and update flags. |
| `shift.r` | `0x003C` | `mnemonic dest <- a, b` | yes | — | Logical right shift by `count mod width`. |
| `shift.r!` | `0x003D` | `mnemonic dest <- a, b` | yes | — | Logical right shift and update flags. |
| `shift.ar` | `0x003E` | `mnemonic dest <- a, b` | yes | — | Arithmetic (sign-preserving) right shift. |
| `shift.ar!` | `0x003F` | `mnemonic dest <- a, b` | yes | — | Arithmetic right shift and update flags. |
| `rotate.l` | `0x0048` | `mnemonic dest <- a, b` | yes | — | Rotate left by `count mod width`. |
| `rotate.l!` | `0x0049` | `mnemonic dest <- a, b` | yes | — | Rotate left and update flags. |
| `rotate.r` | `0x004A` | `mnemonic dest <- a, b` | yes | — | Rotate right by `count mod width`. |
| `rotate.r!` | `0x004B` | `mnemonic dest <- a, b` | yes | — | Rotate right and update flags. |

## Bit manipulation

| Mnemonic | Opcode | Form | Width? | Native? | Notes |
|---|---|---|---|---|---|
| `bit.set` | `0x004C` | `mnemonic dest <- a, b` | yes | — | Set bit `index mod width`. |
| `bit.clear` | `0x004D` | `mnemonic dest <- a, b` | yes | — | Clear bit `index mod width`. |
| `bit.test!` | `0x004E` | `mnemonic a, b` | yes | — | Test bit `index mod width`: Z is set when the bit is clear. |

## Memory

| Mnemonic | Opcode | Form | Width? | Native? | Notes |
|---|---|---|---|---|---|
| `load.u8` | `0x0040` | `mnemonic dest <- [base + off]` | yes | — | Zero-extending 8-bit load from `[base + offset]`. |
| `load.u16` | `0x0041` | `mnemonic dest <- [base + off]` | yes | — | Zero-extending 16-bit load. |
| `load.u32` | `0x0042` | `mnemonic dest <- [base + off]` | yes | — | Zero-extending 32-bit load. |
| `load.u64` | `0x0043` | `mnemonic dest <- [base + off]` | yes | — | 64-bit load. |
| `store.u8` | `0x0044` | `mnemonic [base + off] <- src` | yes | — | 8-bit store to `[base + offset]`. |
| `store.u16` | `0x0045` | `mnemonic [base + off] <- src` | yes | — | 16-bit store. |
| `store.u32` | `0x0046` | `mnemonic [base + off] <- src` | yes | — | 32-bit store. |
| `store.u64` | `0x0047` | `mnemonic [base + off] <- src` | yes | — | 64-bit store. |

## Atomic memory

| Mnemonic | Opcode | Form | Width? | Native? | Notes |
|---|---|---|---|---|---|
| `load.acquire.u8` | `0x0080` | `mnemonic dest <- [base + off]` | yes | — | 8-bit load with acquire ordering; requires a shared, atomic region and natural alignment. |
| `load.acquire.u16` | `0x0081` | `mnemonic dest <- [base + off]` | yes | — | 16-bit acquire load. |
| `load.acquire.u32` | `0x0082` | `mnemonic dest <- [base + off]` | yes | — | 32-bit acquire load. |
| `load.acquire.u64` | `0x0083` | `mnemonic dest <- [base + off]` | yes | — | 64-bit acquire load. |
| `store.release.u8` | `0x0084` | `mnemonic [base + off] <- src` | yes | — | 8-bit store with release ordering; requires a shared, atomic region and natural alignment. |
| `store.release.u16` | `0x0085` | `mnemonic [base + off] <- src` | yes | — | 16-bit release store. |
| `store.release.u32` | `0x0086` | `mnemonic [base + off] <- src` | yes | — | 32-bit release store. |
| `store.release.u64` | `0x0087` | `mnemonic [base + off] <- src` | yes | — | 64-bit release store. |

## Synchronisation

| Mnemonic | Opcode | Form | Width? | Native? | Notes |
|---|---|---|---|---|---|
| `address` | `0x000B` | `mnemonic dest <- symbol` | yes | yes | Place a symbol's code address in a destination. Native backends reject it. |
| `compare.swap` | `0x000D` | `mnemonic dest <- [base+off], expected, desired` | yes | yes | Atomically replace the word with `desired` when it equals `expected`; the old value goes to the destination. |
| `exchange` | `0x0060` | `mnemonic a, b` | — | yes | Swap two full registers. |

## System services

| Mnemonic | Opcode | Form | Width? | Native? | Notes |
|---|---|---|---|---|---|
| `sys.region.map` | `0x0057` | `mnemonic` | — | — | Map a memory region (CAP_MEMORY_MAP). |
| `sys.region.unmap` | `0x0058` | `mnemonic` | — | — | Unmap a memory region (CAP_MEMORY_MAP). |
| `sys.port.read` | `0x0059` | `mnemonic` | — | — | Read an I/O port (CAP_IO_PORT). |
| `sys.port.write` | `0x005A` | `mnemonic` | — | — | Write an I/O port (CAP_IO_PORT). |
| `sys.irq.bind` | `0x005B` | `mnemonic` | — | — | Bind an interrupt handler (CAP_IRQ_MANAGE). |
| `sys.irq.enable` | `0x005C` | `mnemonic` | — | — | Enable an interrupt line (CAP_IRQ_MANAGE). |
| `sys.irq.disable` | `0x005D` | `mnemonic` | — | — | Disable an interrupt line (CAP_IRQ_MANAGE). |
| `sys.context.enter` | `0x005E` | `mnemonic` | — | — | Enter a kernel-managed context (CAP_CONTEXT_SWITCH). |
| `sys.context.return` | `0x005F` | `mnemonic` | — | — | Leave a kernel-managed context (CAP_CONTEXT_SWITCH). |
| `sys.cpu.control` | `0x0067` | `mnemonic` | — | — | CPU control (CAP_CPU_CONTROL). |
| `sys.power` | `0x0068` | `mnemonic` | — | — | Power management (CAP_POWER_CONTROL). |

## Coverage summary

* **Defined instructions:** 97.
* **Native:** 36.
* **Interpreter-only:** 61.

Interpreter-only instructions:

`bit.clear`, `bit.set`, `bit.test!`, `break`, `catch`, `checked.sum`, `dec`, `dec!`, `diffc`, `diffc!`, `extend.s`, `extend.z`, `inc`, `inc!`, `load.acquire.u16`, `load.acquire.u32`, `load.acquire.u64`, `load.acquire.u8`, `load.u16`, `load.u32`, `load.u64`, `load.u8`, `mul`, `neg`, `neg!`, `not`, `not!`, `raise`, `rotate.l`, `rotate.l!`, `rotate.r`, `rotate.r!`, `select`, `shift.ar`, `shift.ar!`, `shift.l`, `shift.l!`, `shift.r`, `shift.r!`, `store.release.u16`, `store.release.u32`, `store.release.u64`, `store.release.u8`, `store.u16`, `store.u32`, `store.u64`, `store.u8`, `sumc`, `sumc!`, `sys.context.enter`, `sys.context.return`, `sys.cpu.control`, `sys.irq.bind`, `sys.irq.disable`, `sys.irq.enable`, `sys.port.read`, `sys.port.write`, `sys.power`, `sys.region.map`, `sys.region.unmap`, `uncatch`
