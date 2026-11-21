# +A Memory Model

**Edition:** ASCENSION +A Core V2.0.0

## 1. No implicit memory

A +A program has no stack, no heap, no global data segment, and no way to
allocate. All memory a program can address has been **mapped by its embedder**
before execution:

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

Bytecode carries a `DATA` section, but it holds constants described by the image
format ([BYTECODE-FORMAT.md](BYTECODE-FORMAT.md) §4); it is **not** automatically
mapped into the address space. There is no instruction that reads the `DATA`
section. This is a deliberate Core decision: mapping policy belongs to whoever
hosts the machine.

## 2. Addresses

An address is a `u64`: the value of a full (`.q`) base register plus a signed
32-bit displacement.

```
load.u32 R0.d <- [R1.q - 4]
```

* The base must be a full GPR. A `.b`, `.w` or `.d` base is a parse error.
* The displacement is in `[-2^31, 2^31 - 1]`. Anything outside is rejected at
  parse time.
* `base + displacement` is computed with overflow checking; an overflow is a
  `MEMORY_ACCESS` fault, not a wrap.

The interpreter and the native backends use the same rule, so a disassembled
`[R1.q - 4]` means the same bytes in every tier.

## 3. Access rules

| Rule | Fault when violated |
|---|---|
| The whole access lies inside exactly one mapped region | `MEMORY_ACCESS` |
| Loads need `readable` | `MEMORY_ACCESS` |
| Stores need `writable` | `MEMORY_ACCESS` |
| `load.acquire.*` / `store.release.*` need `shared` **and** `atomic` | `MEMORY_ACCESS` |
| `*.acquire` / `*.release` addresses must be naturally aligned | `ALIGNMENT` |
| `compare.swap` addresses must be naturally aligned and point at a shared, atomic region | `ALIGNMENT` / `MEMORY_ACCESS` |
| Regions may not overlap | `map_region` returns an error |

Note the two independent alignment concepts:

* **Atomic alignment** is enforced (see the table above).
* **Ordinary unaligned access is not forbidden.** `load.u64` from an odd address
  inside a readable region succeeds. x86-64 permits unaligned scalar access, so
  both the interpreter and the native tiers implement it. Programs that want
  portability should stay naturally aligned.

## 4. Widths and endianness

Loads and stores come in explicit widths: `.u8`, `.u16`, `.u32`, `.u64`. The
width in the mnemonic **must** match the register view
(`load.u32 R0.d <- …`; `store.u8 [R1.q] <- R0.b`), otherwise assembly fails with
`load.u32 requires a 32-bit register view`.

All memory is **little-endian**. A `store.u32` of `0x11223344` writes
`44 33 22 11`.

Loads are **not** widening. A load writes through the destination register's
view, exactly like any other write (see [REGISTERS.md](REGISTERS.md) §2.2):

* `load.u8 R0.b <- [R1.q]` replaces the low byte of `R0` and **preserves bits
  8–63**. It does not zero the rest of the register.
* There is no sign-extending load. To widen a loaded byte, clear the destination
  first (`move R0.q <- 0`) and then load into `.b`; or load and then use
  `extend.s` (which is interpreter-only, see [JIT-AOT.md](JIT-AOT.md)).

A consequence worth stating plainly: `load.u8 R0.q <- [R1.q]` does not assemble.
The load width and the destination view must match, so a byte load needs `.b`,
a word load `.w`, a dword load `.d`, and a qword load `.q`.

## 5. Ordinary vs. atomic access

* `load.uN` / `store.uN` are ordinary accesses. The interpreter is
  single-threaded, so ordering is trivially total; `fence`
  (`acquire`/`release`/`full`) is accepted and is a no-op.
* `load.acquire.uN` / `store.release.uN` additionally require the region to be
  `shared` and `atomic`, and require natural alignment. They exist so that a
  future multi-threaded host has a defined contract. In Core there is only one
  thread, so they behave like ordinary accesses once the checks pass.
* `compare.swap dest <- [base+off], expected, desired` is a single atomic
  read-modify-write on the host, expressed through `cmpxchg` in the native
  backend. It returns the previous value in `dest`.

## 6. What Core does not specify

* **No stack discipline.** The architecture has no stack pointer. There is a
  *call* stack inside the machine, but it is not addressable memory.
* **No heap.** Allocation is a Foundation concern.
* **No memory protection model** beyond the region flags above.
* **No sharing between machines.** Regions belong to one `Avm` instance.

## 7. Interaction with the execution tiers

| Behaviour | Interpreter | JIT | AOT |
|---|---|---|---|
| Ordinary load/store | yes | unsafe library API only | yes |
| Atomic load/store | yes | unsafe library API only | yes |
| `compare.swap` | yes | unsafe library API only | yes |
| Unaligned ordinary access | allowed | allowed (unsafe API) | allowed |
| Fault on unmapped access | `MEMORY_ACCESS` | host signal (unsafe API) | host signal |

Native memory is **direct host memory**, not sandboxed: there are no region
checks, and an invalid address crashes the process with a host signal rather
than a VM fault. See [JIT-AOT.md](JIT-AOT.md) §1.1. Native access also lacks AVM permission, overflow, and alignment checks.
