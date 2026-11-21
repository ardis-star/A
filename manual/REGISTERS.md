# +A Register Architecture

**Edition:** ASCENSION +A Core V1.1.0

## 1. Machine state

The architectural state of a +A machine is exactly:

| Component | Size | Description |
|---|---|---|
| `R0`–`R31` | 32 × 64 bits | General-purpose registers. |
| `FLAGS` | 6 bits | Condition flags (§4). |
| `PC` | word index | Position in the current function's instruction stream. |
| call stack | unbounded in the AVM, capped at 1 048 576 frames | Return points. |
| handler stack | per functioning | Exception targets, each tagged with its owning function. |

There is no stack pointer, frame pointer, or link register in the architecture.
`PC`, `SP`, `FP`, and `FLAGS` are **not** addressable as registers: using `SP`
as an operand is a parse error, not a reserved register.

## 2. Register views

Each register is one 64-bit slot that can be read and written through four
views:

| View | Width | Bytes | Position |
|---|---|---|---|
| `.b` | 8 | 1 | low byte |
| `.w` | 16 | 2 | low word |
| `.d` | 32 | 4 | low dword |
| `.q` | 64 | 8 | whole slot |

`Rn` and `Rn.q` are interchangeable spellings.

### 2.1 Read semantics

A read through a narrower view yields the low bits of the slot, **zero-extended**
to 64 bits. Reading `R0.b` when `R0 = 0x...FF` yields `0xFF`, not `0xFFFF...FF`.
This is deliberately unlike x86, where the high bits of the containing register
are still significant to later 64-bit operations: in +A the read value *is* the
zero-extended value.

### 2.2 Write semantics

| View written | Effect on the slot |
|---|---|
| `.q` | All 64 bits replaced. |
| `.d` | All 64 bits replaced; the value is zero-extended from 32 bits. |
| `.w` | Low 16 bits replaced; bits 16–63 preserved. |
| `.b` | Low 8 bits replaced; bits 8–63 preserved. |

Note the asymmetry: `.d` **clears** the upper 32 bits, while `.b` and `.w`
preserve their neighbours. This matches the x86 rule that a 32-bit write
zero-extends, and it is what the JIT and AOT backends implement
(`write_register` in `rust/src/native.rs`).

## 3. Register constraints

* Index `0`–`31`, written in decimal without redundant leading zeros: `R00` is
  rejected, `R32` is out of range.
* All register operands of one instruction must share a view, except:
  * `exchange`, which takes two full registers;
  * `extend.s` / `extend.z`, whose source is narrower than the destination.
* A memory operand's base must be a full (`.q`) register. The displacement is a
  signed 32-bit value.

## 4. Flags

Six bits in `FLAGS`:

| Bit | Name | Set when |
|---|---|---|
| 0 | `Z` Zero | the result is zero |
| 1 | `C` Carry | the unsigned operation overflowed its width, or the bit shifted out is 1 |
| 2 | `S` Sign | the top bit of the result is set |
| 3 | `O` Overflow | the signed operation overflowed its width |
| 4 | `P` Parity | the low byte of the result has even parity (matches x86 `PF`) |
| 5 | `D` Direction | set by `flag.direction <- backward`, cleared by `forward` |

Rules:

* Only mnemonics spelled with a trailing `!` update flags, plus `compare!` and
  `bit.test!`, which produce flags without a destination.
* A flag-setting result recomputes Z, S, C, O and P, and leaves D untouched.
* `flag.direction` changes D only.
* Divisions never modify flags.
* `mul!` always clears C and O.
* Rotates are like shifts in their flag handling: a rotate by a count
  equivalent to zero is a no-op that clears C; otherwise C receives the last bit
  rotated out (the bit that moves into the vacated position). See
  `shift_result` in `rust/src/avm.rs` for the exact bit.
* Shifts and rotates reduce their count modulo the operation width first, so a
  count equal to the width behaves as a count of zero.

The JIT and AOT backends materialise `FLAGS` as a 64-bit word in the native
frame using the same bit positions, so a native program and the interpreter see
identical flag values (see [JIT-AOT.md](JIT-AOT.md)).

## 5. Flags vs. tier fidelity

The interpreter, the JIT, and AOT code all compute Z, S, C, O and P from the
same rules. The differential tests in `rust/tests/regressions.rs`
(`execution_tiers_agree_on_a_representative_matrix`) compare flag-visible
outcomes across tiers. Only the 36 instructions listed as native in
[INSTRUCTION-REFERENCE.md](INSTRUCTION-REFERENCE.md) can be compared this way;
the rest are interpreter-only and reported as such.

## 6. Register persistence

Registers are machine state and persist across `call`, `return`, `raise`, and
`catch`. There is **no** automatic save/restore: a callee overwrites whatever it
touches. V1 specifies no calling convention beyond "arguments and results go in
registers chosen by the author", because +A has one function in the emitted
image and no parameter mechanism. A stack-based convention is a *convention*
that a higher layer (Foundation) must define; it is not part of Core.
