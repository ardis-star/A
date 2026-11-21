# +A Bytecode Format (`.avm`)

**Format:** A+BC version 1.0 (major 1, minor 0 or 1)
**Edition:** ASCENSION +A Core V2.0.0

An `.avm` file is a self-contained, position-independent container for one
program unit. It is verified before it is loaded or executed.

## 1. Design rules

1. **Little-endian** throughout. Every multi-byte field is little-endian.
2. **Bounds-checked at every step.** The decoder refuses any image larger than
   256 MiB, any function body over 16 MiB, and any single instruction payload
   over 16 MiB.
3. **Self-describing.** A 40-byte header points at a directory of 32-byte
   section entries; each entry gives a type, flags, an absolute payload offset,
   a length, an alignment and a CRC-32C.
4. **Deterministic.** The encoder iterates over ordered maps, so the same source
   always yields the same bytes. Section payloads are padded to 8 bytes.
5. **No pointers.** The format contains no absolute addresses. Control flow is
   expressed as block identifiers resolved through the `BLOCKS` section.

## 2. Header (40 bytes)

| Offset | Size | Field | Notes |
|---|---|---|---|
| 0 | 4 | magic | ASCII `A+BC` (`41 2B 42 43`). |
| 4 | 2 | major version | `1`. Other values are rejected. |
| 6 | 2 | minor version | `0` (no control flow) or `1` (with `BLOCKS`). Values above `1` are rejected. |
| 8 | 2 | header size | `40`. |
| 10 | 2 | flags | Reserved; must be `0`. |
| 12 | 8 | total size | Total image length in bytes. Must match the file length exactly. |
| 20 | 8 | section directory offset | `40` in images produced by this toolchain. |
| 28 | 4 | section count | Number of 32-byte directory entries. |
| 32 | 4 | entry function | Function ID to start at, or `0xFFFFFFFF` for a library image. |
| 36 | 4 | reserved | `0`. |

## 3. Section directory

Each entry is 32 bytes:

| Offset | Size | Field |
|---|---|---|
| 0 | 4 | section type |
| 4 | 4 | flags |
| 8 | 8 | payload offset (absolute, aligned to the entry’s alignment) |
| 16 | 8 | payload length |
| 24 | 4 | payload alignment (power of two; `8` in assembler output) |
| 28 | 4 | CRC-32C of the payload |

Defined section types:

| Type | Name | Required | Contents |
|---|---|---|---|
| 1 | `META` | no | Producer string (§4). |
| 3 | `DATA` | yes | Read-only constant data records (§5). |
| 4 | `CODE` | yes | Function descriptors followed by instruction streams (§6). |
| 5 | `SYMS` | no | Exported symbol names (§7). |
| 7 | `EXCP` | no | Exception-handler ranges (never emitted by this assembler). |
| 8 | `CAPS` | yes | Capability requirements (§8). |
| 9 | `BLOCKS` | conditional | Block map, present only when the program has control flow (§9). |

Section flags: bit 0 (`0x1`) marks a **required** section. Only `BLOCKS` may
carry the required flag; any other section with the flag set is rejected, and
`BLOCKS` without the flag is rejected. A duplicated section type,
a missing required section, a misaligned payload offset, or a bad CRC is a
verification error. The assembler always emits `META`, `DATA`, `CODE`,
`SYMS`, and `CAPS`, plus `BLOCKS` when the program has control flow.

## 4. `META`

An opaque producer string, NUL-terminated: `+A assembler 2.0.0\0`. The
verifier checks only its CRC; the content is informational. (Earlier
releases wrote `+A assembler 0.1\0`; the string changed in V2.0.0 without
affecting verification — the content is not interpreted by the verifier.)

## 5. `DATA`

```
u32 record_count
record_count × { u32 region_id; u64 data_offset; u64 length; u32 flags }  (24 bytes each)
```

The verifier checks that the payload length matches the record count and that
no record range overflows. The assembler always emits a zero-length `DATA`
section (also accepted as empty), and no instruction reads it; mapping policy belongs to
the embedder (see [MEMORY-MODEL.md](MEMORY-MODEL.md)).

## 6. `CODE`

```
u32 function_count
function_count × function descriptor      (32 bytes each, from CODE + 4)
instruction streams, in function order
```

Function descriptor (32 bytes; all offsets relative to the start of `CODE`):

| Offset | Size | Field |
|---|---|---|
| 0 | 4 | function ID |
| 4 | 4 | flags (must be `0`) |
| 8 | 8 | code offset |
| 16 | 8 | code length in bytes |
| 24 | 4 | maximum stack frame, in bytes |
| 28 | 2 | argument count (at most 8) |
| 30 | 2 | result count (at most 2) |

In images produced by this toolchain the descriptor table is a single 32-byte
entry and the instruction stream starts 36 bytes into `CODE` (4-byte count +
32-byte descriptor), i.e. `CODE + 36` for function 0. The header's entry
field is `0` when the program exports a label and `0xFFFFFFFF` for a library
image.

### 6.1 Instruction stream

```
u16 opcode
u8  operand_count
u8  width           0 = none, else view code (1=.b, 2=.w, 3=.d, 4=.q)
uleb128 payload_length
payload_length bytes of operands, in operand order
```

Operand encoding (tag byte, then the operand):

| Tag | Operand | Encoding |
|---|---|---|
| 0 | register | 1 byte: `index \| view << 5`; view 0=`.b`, 1=`.w`, 2=`.d`, 3=`.q` |
| 1 | unsigned immediate | unsigned LEB128 |
| 2 | signed immediate | signed LEB128 |
| 4 | label reference | unsigned LEB128 block ID |
| 5 | memory | 1 register byte, then a signed LEB128 displacement |
| 6 | symbol | unsigned LEB128 symbol index |
| 7 | predicate | unsigned LEB128 predicate code (0 zero … 9 odd) |

Any other tag, a payload whose length disagrees with its operands, or an
operand count that contradicts the instruction shape is a verification error.

## 7. `SYMS`

```
u32 count
count × { u16 name_length; name bytes; u32 function_id }
```

Names are unique and ASCII. `address` operands carry a **symbol index** into
this table, which is why the assembler must emit a `SYMS` section for a program
that uses `address`.

## 8. `CAPS`

```
u32 record_count
record_count × { u32 capability_bits }
```

An empty payload is accepted and emitted by the assembler. Nonempty records
are combined into a global capability mask. The
verifier rejects an image whose `CAPS` payload contains bytes beyond the last
record: trailing data in a capability section is an error, not padding.

## 9. `BLOCKS`

```
u32 count
count × { u32 function_id; u32 block_id; u64 code_offset }
```

Block offsets are relative to the start of their function’s instruction stream. The verifier refuses
to execute an image whose block targets cannot be represented
(`unresolved_control_targets`), rather than guessing.

## 10. Verification pipeline

`verify_image` performs, in order: header and magic checks → directory and CRC
checks → `CODE`/`SYMS`/`BLOCKS`/`EXCP`/`DATA` structural checks → per-function
decode and operand checks → capability validation. `decode_image` performs the
structural half only, which is what `+a inspect` uses. See
[VERIFICATION.md](VERIFICATION.md) for the rule list.

## 11. Compatibility

A reader must reject any image whose **major** version is not `1`. The minor
version may be raised for additive changes; V2.0.0 readers accept minor `0`
and `1`. There are currently no other producers of `A+BC` images.
