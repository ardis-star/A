# +A Verification Rules

**Edition:** ASCENSION +A Core V1.1.0

The verifier is the gate between a byte sequence and execution. Nothing executes
— in the interpreter, the JIT, or AOT code — without passing it. This document
lists the rules in the order they are applied. Every rule has at least one test
in `rust/tests/verifier.rs` or `rust/tests/regressions.rs`; the rule text below
is the message the verifier actually prints.

`+a verify <image>` applies the whole pipeline. `+a inspect <image>` applies only
the structural half and then prints section details, which is why `inspect` can
succeed on an image `verify` would refuse (for example one with unresolved
control flow).

## 1. Container rules

| Rule | Message |
|---|---|
| The image is at least 40 bytes | `truncated image header` |
| Magic is `A+BC` | `invalid image magic` |
| Major version is `1` | `unsupported bytecode version` |
| Header size is 40 | `invalid header size` |
| Header flags and reserved word are zero | `reserved header fields must be zero` |
| Declared total size equals the buffer length | `declared file size does not match image` |
| Section directory does not overlap the header | `section directory overlaps image header` |
| Section count is representable and within the format ceiling | `section count is not representable`, `section count exceeds v1 format ceiling` |
| Each directory entry fits in the buffer | `directory entry overflow` |
| Each payload offset is 8-byte aligned | `section offset violates required alignment` |
| Section payloads do not overlap and stay in the image | `directory index overflow`, `section directory size overflow` |
| No duplicate section types | rejected during directory parsing |
| Required sections are present and marked required | `unknown required section`, `unknown required section flags` |
| CRC-32C matches each payload | rejected during directory parsing |
| Image size is at most 256 MiB | `image exceeds v1 format ceiling` |

## 2. `CODE` rules

| Rule | Message |
|---|---|
| Function count is present and representable | `truncated CODE function count`, `function count is not representable` |
| Descriptor table fits | `function descriptor table overflow`, `truncated function descriptor table` |
| Function IDs are dense and ordered | `function IDs must be dense and ordered` |
| Function flags are known | `unknown function flags` |
| Code length within the per-function ceiling | `function code exceeds v1 format ceiling` |
| Code does not overlap the descriptor table or another function | `function code overlaps descriptor table`, `function code ranges overlap` |
| Instruction count within the ceiling | `instruction count exceeds v1 format ceiling` |
| Instruction stream ends exactly on a record boundary | `instruction stream does not end on a record boundary` |
| Entry function, if any, is inside the table | `entry-function ID is outside function table` |
| An empty `CODE` section cannot declare an entry | `empty CODE section cannot have an entry function` |

## 3. Instruction rules

| Rule | Message |
|---|---|
| Instruction header (`opcode`, `count`, `width`, `len`) is present | `truncated instruction header` |
| Opcode is defined | `opcode has invalid operand count` / verification failure naming the opcode |
| Operand count matches the instruction shape | `opcode has invalid operand count` |
| Operand width code agrees with the opcode | `opcode and instruction width code disagree` |
| Width-taking opcodes carry a width | `opcode requires a width` |
| Width code is 0–4 | `instruction has invalid width code` |
| Payload is fully consumed | `operand payload was not consumed exactly` |
| Operand count does not exceed the payload | `operand count exceeds payload` |
| Register view bits are in range | `register operand has reserved view bits` |
| Immediates fit the operation width | `immediate does not fit instruction width` |
| Exchange operands are full GPRs | `exchange requires full GPRs` |
| Extension operands are registers | `extension operands must be registers` |
| Memory base is a full GPR | `memory base must be a full GPR` |
| Memory displacement fits 32 bits | `memory offset does not fit 32 bits` |
| Memory access width matches the register view | `memory opcode register view does not match width` |
| Predicates are known | `unknown predicate operand` |

## 4. Control-flow rules

| Rule | Message |
|---|---|
| Every branch target is mapped by `BLOCKS` | `branch target block N is not mapped` |
| A block offset lands on an instruction boundary | `block target does not point to an instruction boundary` |
| Block IDs are unique within a function | `duplicate block ID in function` |
| Two block IDs may not share an instruction | `multiple block IDs map to the same instruction` |
| Block map function IDs are inside the table | `block map function ID is outside function table` |
| Entry block 0 maps to the first instruction (minor version 1) | `entry block zero must map to the first instruction` |
| No unresolved control targets remain | `block target offsets are not represented in this image format; control-flow image cannot be execution-verified` |
| `uncatch` never pops below the handlers a function pushed | `handler stack underflow in function N` |

The last rule in the table above is what makes an image *execution-verifiable*:
minor version 0 images can carry branch targets that the format cannot resolve,
and `verify_image` refuses them rather than guessing. This is the documented,
intentional behaviour behind `unresolved_control_targets`.

## 5. `SYMS`, `CAPS`, `EXCP`, `DATA` rules

| Rule | Message |
|---|---|
| `SYMS` payload is fully consumed | `trailing bytes in SYMS section` |
| Symbol names are unique and ASCII | `duplicate symbol name`, `symbol name must be ASCII` |
| Symbol function IDs are inside the table | `symbol function ID is outside function table` |
| `CAPS` payload length equals the record count exactly | `CAPS section length does not match record count` |
| `EXCP` ranges are non-empty and ordered | `exception protected range is empty or inverted`, `exception protected ranges overlap`, `exception handler lies inside its protected range` |
| `DATA` records fit in the section | `DATA section length does not match record count`, `data record range overflow` |

The `CAPS` rule is deliberately an **equality** check. Trailing bytes in a
capability section are an error, not padding: an image that appends data after
the declared records is rejected (`caps_section_with_trailing_bytes_is_rejected`).

## 6. Capability rules

An image whose functions use any `sys.*` instruction must carry a `CAPS` record
for the corresponding capability, or verification fails with
`function N uses sys.* without required capability`. `every_system_service_requires_a_capability`
in `rust/tests/regressions.rs` checks all eleven services.

## 7. What the verifier does *not* promise

* It does not prove termination or memory safety of the *program*; it proves
  that the *image* is well formed enough to execute deterministically.
* It does not check that a program's memory accesses will succeed — there is no
  memory at verification time. Unmapped accesses are runtime faults.
* It does not enforce function boundaries for branches. `BLOCKS` records carry a
  function ID, but branch targets are resolved against the *current* function's
  block map only, so a target belonging to another function fails with
  `branch target block N is not mapped` rather than being redirected. The
  consequence is a conservative limitation, not an unsound one: a
  cross-function transfer must use `call`/`tailcall`, which is the only
  mechanism the language offers. Note that this toolchain only ever emits a
  single function per image, so the case cannot arise from `+a` output.
