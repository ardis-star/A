# The ASCENSION Virtual Machine (AVM)

**Edition:** ASCENSION +A Core V1.1.0

The AVM is the reference execution tier: an interpreter for `A+BC` bytecode.
It is the definition of correct behaviour — the JIT and AOT backends are tested
against it, not the other way round.

## 1. Loading

```rust
let mut machine = Avm::load(&image)?;   // verifies, then loads
let result = machine.run()?;
```

* `Avm::load` runs the full verifier ([VERIFICATION.md](VERIFICATION.md)) and
  refuses anything that does not verify.
* `Avm::load_unverified` skips verification. It exists for fuzzing and for
  tooling that inspects malformed images deliberately; it is **not** a way to
  run untrusted bytecode faster.
* Construction is inert. No instruction executes until `run` is called, and
  loading performs no I/O and allocates no host memory on behalf of the program.

## 2. Initial state

| State | Initial value |
|---|---|
| `R0`–`R31` | all zero |
| `FLAGS` | zero (so `parity`/`even` reads true) |
| `PC` | 0, in the entry function |
| call stack | empty |
| handler stack | empty |
| memory regions | none |
| privileged | `false` |
| capabilities | `0` |
| step budget | 10 000 000 instructions |

An empty call stack means a `return` ends the program with `Returned` rather
than faulting — this matches the native backends, which push a sentinel return
address.

## 3. Memory

The AVM has **no implicit memory**. A program cannot allocate, cannot address the
host, and cannot read outside a region the embedder mapped explicitly:

```rust
machine.map_region(MemoryRegion {
    base: 0x1000,
    bytes: vec![0; 4096],
    readable: true,
    writable: true,
    shared: false,
    atomic: false,
})?;
```

Every access is checked at run time:

| Condition | Fault |
|---|---|
| The access spans more than one region, or no region | `MEMORY_ACCESS` |
| The region lacks the required `readable`/`writable` permission | `MEMORY_ACCESS` |
| Address arithmetic overflows `u64` | `MEMORY_ACCESS` |
| An `*.acquire`/`*.release` access is misaligned, or the region is not `shared` **and** `atomic` | `ALIGNMENT` / `MEMORY_ACCESS` |

Regions may not overlap; `map_region` rejects an overlapping region. Bytes are
little-endian. Loads are zero-extending: `load.u8` into `.q` leaves bits 8–63
clear.

Because there is no implicit memory, **a Core program that touches memory must
be given regions by its embedder**. The CLI does not map any regions, so a
memory-touching program run with `+a` faults with `MEMORY_ACCESS` — which is the
honest outcome, not a bug.

## 4. Execution

`run` executes instructions until one of:

1. `stop` → `Ok(RunResult::Stopped(status))`, where `status` is the full 64-bit
   `R0`.
2. `return` with an empty call stack, or the program counter running past the
   end of the function → `Ok(RunResult::Returned)`.
3. a fault → `Err(AvmError)`, with the program counter restored to the faulting
   instruction.
4. the step budget running out → `Err(AvmError::StepLimit)`.

Faults do not commit partial effects: `checked.sum` that overflows leaves the
destination unchanged, and a faulting instruction can be retried after the
machine is repaired (for example after mapping a region).

### 4.1 Step budget and resumption

The budget is consumed by every instruction, including branches and calls. When
it runs out the machine is left **restartable**: `pc` points at the next
instruction that would have executed. Increasing the limit with
`with_step_limit` and calling `run` again resumes rather than repeating.
`step_limit_is_a_restartable_budget` in `rust/tests/regressions.rs` pins this.

## 5. Faults

`AvmError` variants, and how they are reported:

| Variant | Rendered as | Cause |
|---|---|---|
| `Verification` | `verification failed: …` | image rejected before execution |
| `MissingEntry` | `image has no entry function` | library image executed |
| `UnsupportedInstruction` | `unsupported AVM opcode 0x…` | opcode outside the table |
| `InvalidOperand` | `invalid verified operand: …` | malformed verified operand |
| `IntegerOverflow` | `INTEGER_OVERFLOW` | `checked.sum`, or `MIN / -1` |
| `DivByZero` | `DIVIDE_BY_ZERO` | division by zero |
| `MemoryAccess` | `MEMORY_ACCESS at 0x… for N bytes` | unmapped or forbidden access |
| `Alignment` | `ALIGNMENT at 0x…; required alignment N` | misaligned atomic access |
| `Breakpoint` | `BREAKPOINT` | `break` |
| `PrivilegeViolation` | `PRIVILEGE_VIOLATION` | `sys.*` without privilege or capability |
| `UnsupportedService` | `UNSUPPORTED_SERVICE` | privileged, capable, but the service is not implemented |
| `InvalidHandler` | `INVALID_HANDLER` | `raise` with no handler in this function |
| `CallStackLimit` | `AVM call stack limit exceeded` | more than 1 048 576 nested calls |
| `StepLimit` | `AVM instruction step limit exceeded` | budget exhausted |

## 6. Exceptions

`catch -> handler` pushes `(current_function, handler)`; `uncatch` pops;
`raise code` transfers to the innermost handler **registered by the current
function** and puts `code` in `R0`.

Handlers are function-scoped. A `raise` in function B never jumps to a handler
that function A registered. When a `raise` unwinds past a frame, handlers owned
by dead frames are discarded. If no handler belongs to the current function, the
program faults with `INVALID_HANDLER`. This behaviour is pinned by
`raise_only_targets_a_handler_in_the_raising_function` and
`raise_without_a_local_handler_faults`.

## 7. Privilege and `sys.*`

Every `sys.*` instruction is gated twice:

1. the machine must be `privileged`;
2. the machine's capability bits must include the service's requirement.

A non-privileged machine faults with `PRIVILEGE_VIOLATION` even if the
capability bits are present. A privileged, capable machine currently faults with
`UNSUPPORTED_SERVICE` for every service, because **no host-side service is
implemented in Core**. That is deliberate: it keeps the AVM isolated and makes
the eventual Foundation implementation an addition rather than a change of
behaviour. See [SYS-ARCHITECTURE.md](SYS-ARCHITECTURE.md).

Capabilities:

| Bit | Name | Services |
|---|---|---|
| 0 | `CAP_MEMORY_MAP` | `sys.region.map`, `sys.region.unmap` |
| 1 | `CAP_IO_PORT` | `sys.port.read`, `sys.port.write` |
| 2 | `CAP_IRQ_MANAGE` | `sys.irq.bind`, `sys.irq.enable`, `sys.irq.disable` |
| 3 | `CAP_CONTEXT_SWITCH` | `sys.context.enter`, `sys.context.return` |
| 4 | `CAP_CPU_CONTROL` | `sys.cpu.control` |
| 5 | `CAP_POWER_CONTROL` | `sys.power` |

The verifier requires an image that uses `sys.*` to carry a matching `CAPS`
record, so an unprivileged program that uses a service is rejected at load time
rather than at first execution.

## 8. Determinism and limits

* Execution is single-threaded and deterministic for a given image, memory
  layout and step budget.
* The call stack is capped at 1 048 576 frames (`CALL_STACK_LIMIT`).
* There is no instruction-level timeout; use the step budget.
* The interpreter does not use the host stack, so deeply recursive programs
  cannot smash the host stack.

## 9. Not implemented in Core

* No `sys.*` host service is implemented (see §7).
* No threads, no asynchronous interrupts, no timers.
* No memory allocation facility. Regions come from the embedder.
