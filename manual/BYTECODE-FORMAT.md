# +A Bytecode Format (`.avm`)

**Format:** A+BC version 1.0 (major 1, minor 0)
**Edition:** ASCENSION +A Core V1.1.0

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
| 6 | 2 | minor version | `0`. |
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
| 8 | 8 | payload offset (absolute, 8-byte aligned) |
| 16 | 8 | payload length |
| 24 | 4 | payload alignment (`8`) |
| 28 | 4 | CRC-32C of the payload |

Defined section types:

| Type | Name | Required | Contents |
|---|---|---|---|
| 1 | `META` | yes | Metadata records (§4). |
| 3 | `DATA` | no | Read-only constant data. |
| 4 | `CODE` | yes | Function descriptors followed by instruction streams. |
| 5 | `SYMS` | no | Exported symbol names (§6). |
| 8 | `CAPS` | no | Capability requirements (§7). |
| 9 | `BLOCKS` | conditional | Block map, present only when the program has control flow. |

Section flags: bit 0 (`0x1`) marks a **required** section. An unknown or
duplicated section type, a missing required section, a misaligned payload
offset, or a bad CRC is a verification error.

## 4. `META`

A 32-bit record count followed by that many records of
`u16 length`, `length` bytes of UTF-8. Keys used by this toolchain include the
producer name and version.

## 5. `CODE`

```
u32 function_count
function_count × function descriptor      (32 bytes each, from CODE + 4)
instruction streams, in function order
```

Function descriptor (32 bytes; all offsets relative to the start of `CODE`):

| Offset | Size | Field |
|---|---|---|
| 0 | 4 | function ID |
| 4 | 4 | flags (bit 0 = exported) |
| 8 | 8 | code offset |
| 16 | 8 | code length in bytes |
| 24 | 4 | maximum stack frame, in bytes |
| 28 | 2 | argument count |
| 30 | 2 | reserved |

In images produced by V1 the descriptor table is a single 32-byte entry and the
instruction stream starts 36 bytes into `CODE` (4-byte count + 32-byte
descriptor), i.e. `CODE + 36` for function 0. Exactly **one** function carries
the exported flag, and its stream must begin with the program's first
instruction; otherwise the image is a library image and the header's entry field
is `0xFFFFFFFF`.

### 5.1 Instruction stream

```
u16 opcode
u8  operand_count
u8  width           0 = none, else view code + 1 (1=.b, 2=.w, 3=.d, 4=.q)
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

## 6. `SYMS`

```
u32 count
count × { u16 name_length; name bytes; u32 function_id }
```

Names are unique and ASCII. `address` operands carry a **symbol index** into
this table, which is why the assembler must emit a `SYMS` section for a program
that uses `address`.

## 7. `CAPS`

```
u32 record_count
record_count × { u16 length; required capability bits }
```

Each record states the capability bits a function requires to run. The
verifier rejects an image whose `CAPS` payload contains bytes beyond the last
record: trailing data in a capability section is an error, not padding.

## 8. `BLOCKS`

```
u32 count
count × { u32 function_id; u32 block_id; u64 code_offset }
```

Blocks are resolved to absolute offsets inside `CODE`. The V1 verifier refuses
to execute an image whose block targets cannot be represented
(`unresolved_control_targets`), rather than guessing.

## 9. Verification pipeline

`verify_image` performs, in order: header and magic checks → directory and CRC
checks → `META`/`CODE`/`SYMS`/`CAPS`/`BLOCKS` structural checks → per-function
decode and operand checks → capability validation. `decode_image` performs the
structural half only, which is what `+a inspect` uses. See
[VERIFICATION.md](VERIFICATION.md) for the rule list.

## 10. Compatibility

A reader must reject any image whose **major** version is not `1`. The minor
version may be raised for additive changes; V1 readers accept only minor `0`.
There are currently no other producers of `A+BC` images.
