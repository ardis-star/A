use std::io::Write;
use std::process::{Command, Stdio};

use ascension_asm::avm::{Avm, AvmError, MemoryRegion, RunResult};
use ascension_asm::bytecode::{
    EncodeError, MAX_INSTRUCTIONS, crc32c, decode_sleb, decode_uleb, encode, sleb, uleb,
};
use ascension_asm::jit::run_jit;
use ascension_asm::lexer::{TokenKind, lex};
use ascension_asm::model::{Instruction, Program};
use ascension_asm::native::{compile_executable, emit_assembly};
use ascension_asm::parser::{ParseError, parse};
use ascension_asm::verifier::{decode_image, verify_image};
use ascension_asm::{AssemblyError, assemble};

const SOURCE: &str = include_str!("golden/canonical.a");

fn assert_parse_error(source: &str, message: &str) {
    let error = parse(source).unwrap_err().to_string();
    assert!(error.contains(message), "expected {message:?} in {error:?}");
}

fn assert_assembly_error(source: &str, message: &str) {
    let error = assemble(source).unwrap_err().to_string();
    assert!(error.contains(message), "expected {message:?} in {error:?}");
}

fn u16_at(data: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(data[offset..offset + 2].try_into().unwrap())
}

fn u32_at(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap())
}

fn u64_at(data: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(data[offset..offset + 8].try_into().unwrap())
}

fn decode_hex(value: &str) -> Vec<u8> {
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let digits = std::str::from_utf8(pair).unwrap();
            u8::from_str_radix(digits, 16).unwrap()
        })
        .collect()
}

fn code_offset(image: &[u8]) -> usize {
    u64_at(image, 40 + 2 * 32 + 8) as usize
}

fn decode_instruction_opcodes(code_section: &[u8], code_length: usize) -> Vec<u16> {
    let mut position = 36;
    let mut opcodes = Vec::new();
    while position < 36 + code_length {
        opcodes.push(u16_at(code_section, position));
        let (payload_length, payload_end) = decode_uleb(code_section, position + 4).unwrap();
        position = payload_end + payload_length as usize;
    }
    opcodes
}

// Original Python test: LexerTests.test_comments_and_swap_are_tokens
#[test]
fn comments_and_swap_are_tokens() {
    let tokens = lex("exchange R1 <-> R2 ;; comment\n").unwrap();
    let kinds: Vec<_> = tokens.iter().map(|token| token.kind).collect();
    assert_eq!(
        kinds,
        [
            TokenKind::Identifier,
            TokenKind::Identifier,
            TokenKind::Swap,
            TokenKind::Identifier,
            TokenKind::Newline,
            TokenKind::Eof,
        ]
    );
}

// Original Python test: LexerTests.test_all_token_types
#[test]
fn all_token_types() {
    let tokens = lex("label:: move R0 <- -0x10, [R1 + 8] -> target <-> ;; note\n").unwrap();
    let kinds: Vec<_> = tokens
        .iter()
        .filter(|token| !matches!(token.kind, TokenKind::Newline | TokenKind::Eof))
        .map(|token| token.kind)
        .collect();
    assert_eq!(
        kinds,
        [
            TokenKind::Identifier,
            TokenKind::Label,
            TokenKind::Identifier,
            TokenKind::Identifier,
            TokenKind::Arrow,
            TokenKind::Number,
            TokenKind::Comma,
            TokenKind::LeftBracket,
            TokenKind::Identifier,
            TokenKind::Plus,
            TokenKind::Number,
            TokenKind::RightBracket,
            TokenKind::Branch,
            TokenKind::Identifier,
            TokenKind::Swap,
        ]
    );
}

// Original Python test: LexerTests.test_unexpected_character_raises_lex_error
#[test]
fn unexpected_character_raises_lex_error() {
    assert!(
        lex("move R0 <- @invalid\n")
            .unwrap_err()
            .to_string()
            .contains("unexpected character")
    );
    assert!(
        lex("move R0 <- $100\n")
            .unwrap_err()
            .to_string()
            .contains("unexpected character")
    );
}

// Original Python test: ParserTests.test_labels_and_operand_model
#[test]
fn labels_and_operand_model() {
    let program = parse(SOURCE).unwrap();
    assert_eq!(program.labels.get("boot"), Some(&0));
    assert_eq!(program.labels.get("done"), Some(&5));
    assert_eq!(program.instructions.len(), 6);
}

// Original Python test: ParserTests.test_export_syntax_variants
#[test]
fn export_syntax_variants() {
    let first = parse("export boot\nboot::\n    return\n").unwrap();
    assert!(first.exports.contains("boot"));
    let second = parse("export boot::\nboot::\n    return\n").unwrap();
    assert!(second.exports.contains("boot"));
}

// Original Python test: ParserTests.test_rejects_duplicate_unit
#[test]
fn rejects_duplicate_unit() {
    assert_parse_error("unit first\nunit second\n", "duplicate unit");
}

// Original Python test: ParserTests.test_rejects_ambiguous_width
#[test]
fn rejects_ambiguous_width() {
    assert_parse_error("sum R0.d <- R1.q, R2.q\n", "widths must match");
}

// Original Python test: ParserTests.test_exchange_requires_distinct_full_registers
#[test]
fn exchange_requires_distinct_full_registers() {
    assert_parse_error("exchange R1 <-> R1\n", "must differ");
    assert_parse_error("exchange R1.b <-> R2.b\n", "full GPRs");
}

// Original Python test: ParserTests.test_memory_width_and_addressing_are_explicit
#[test]
fn memory_width_and_addressing_are_explicit() {
    assert_parse_error("load.u8 R0.q <- [R1.q]\n", "requires a 8-bit");
    assert_parse_error("store.u64 [R1.d] <- R0.q\n", "full GPR");
    assert_parse_error("move R0.b <- 256\n", "does not fit 8");
}

// Original Python test: ParserTests.test_register_index_and_view_validation
#[test]
fn register_index_and_view_validation() {
    let program = parse("move R0.b <- 1\nmove R31.q <- 2\n").unwrap();
    assert_eq!(program.instructions.len(), 2);
    assert_parse_error("move R32 <- 1\n", "out of range");
    assert_parse_error("move R00 <- 1\n", "expected R0 through R31");
    assert_parse_error("move R01 <- 1\n", "expected R0 through R31");
    assert_parse_error("move R0.x <- 1\n", "invalid register view");
    assert_parse_error("move R0.b.w <- 1\n", "invalid register view");
    assert_parse_error("move SP <- 1\n", "expected R0 through R31");
}

// Original Python test: ParserTests.test_memory_offset_validation
#[test]
fn memory_offset_validation() {
    let program = parse("load.u64 R0.q <- [R1.q + 0]\nload.u64 R0.q <- [R1.q + 0x7FFF_FFFF]\nload.u64 R0.q <- [R1.q + -0x8000_0000]\nload.u64 R0.q <- [R1.q + -8]\n").unwrap();
    assert_eq!(program.instructions.len(), 4);
    assert_parse_error(
        "load.u64 R0.q <- [R1.q + 0x1_0000_0000]\n",
        "memory offset does not fit 32 bits",
    );
    assert_parse_error(
        "load.u64 R0.q <- [R1.q + -0x8000_0001]\n",
        "memory offset does not fit 32 bits",
    );
}

// Original Python test: ParserTests.test_immediate_bounds_for_all_widths
#[test]
fn immediate_bounds_for_all_widths() {
    parse("move R0.b <- -128\nmove R0.b <- 255\n").unwrap();
    assert_parse_error("move R0.b <- 256\n", "does not fit 8 bits");
    assert_parse_error("move R0.b <- -129\n", "does not fit 8 bits");
    parse("move R0.w <- -32768\nmove R0.w <- 65535\n").unwrap();
    assert_parse_error("move R0.w <- 65536\n", "does not fit 16 bits");
    assert_parse_error("move R0.w <- -32769\n", "does not fit 16 bits");
    parse("move R0.d <- -0x8000_0000\nmove R0.d <- 0xFFFF_FFFF\n").unwrap();
    assert_parse_error("move R0.d <- 0x1_0000_0000\n", "does not fit 32 bits");
    assert_parse_error("move R0.d <- -0x8000_0001\n", "does not fit 32 bits");
    parse("move R0.q <- -0x8000_0000_0000_0000\nmove R0.q <- 0xFFFF_FFFF_FFFF_FFFF\n").unwrap();
    assert_parse_error(
        "move R0.q <- 0x1_0000_0000_0000_0000\n",
        "does not fit 64 bits",
    );
}

// Original Python test: ParserTests.test_compare_requires_register_for_width
#[test]
fn compare_requires_register_for_width() {
    parse("compare! R0.q, 10\ncompare! 10, R0.q\ncompare! R0.b, R1.b\n").unwrap();
    assert_parse_error(
        "compare! 10, 20\n",
        "requires at least one register to determine width",
    );
}

// Original Python test: ParserTests.test_malformed_number_literal
#[test]
fn malformed_number_literal() {
    assert_parse_error("move R0.q <- 007\n", "invalid number literal");
}

// Original Python test: EncodingTests.test_encodes_canonical_header_and_required_sections
#[test]
fn encodes_canonical_header_and_required_sections() {
    let image = assemble(SOURCE).unwrap();
    assert_eq!(&image[0..4], b"A+BC");
    assert_eq!(
        (u16_at(&image, 4), u16_at(&image, 6), u16_at(&image, 8)),
        (1, 1, 40)
    );
    assert_eq!(u64_at(&image, 12), image.len() as u64);
    assert_eq!(u64_at(&image, 20), 40);
    assert_eq!(u32_at(&image, 28), 6);
    assert_eq!(u32_at(&image, 32), 0);
    assert_eq!(u32_at(&image, 36), 0);
    let section_types: Vec<_> = (0..6)
        .map(|index| u32_at(&image, 40 + index * 32))
        .collect();
    assert_eq!(section_types, [1, 3, 4, 5, 8, 9]);
}

// Original Python test: EncodingTests.test_header_entry_function_id_library_vs_exported
#[test]
fn header_entry_function_id_library_vs_exported() {
    let exported = assemble("export main\nmain::\n    return\n").unwrap();
    assert_eq!(u32_at(&exported, 32), 0);
    let library = assemble("main::\n    return\n").unwrap();
    assert_eq!(u32_at(&library, 32), u32::MAX);
}

// Original Python test: EncodingTests.test_section_checksums_are_crc32c
#[test]
fn section_checksums_are_crc32c() {
    assert_eq!(crc32c(b"123456789"), 0xe306_9283);
    let image = assemble(SOURCE).unwrap();
    for index in 0..6 {
        let offset = u64_at(&image, 40 + index * 32 + 8) as usize;
        let length = u64_at(&image, 40 + index * 32 + 16) as usize;
        assert_eq!(u32_at(&image, 40 + index * 32 + 24), 8);
        assert_eq!(
            u32_at(&image, 40 + index * 32 + 28),
            crc32c(&image[offset..offset + length])
        );
    }
}

// Original Python test: EncodingTests.test_label_aliases_share_a_block_target
#[test]
fn label_aliases_share_a_block_target() {
    let image = assemble("entry::\nalias::\n    jump -> alias\n").unwrap();
    let offset = code_offset(&image);
    assert_eq!(&image[offset + 36 + 5..offset + 36 + 7], &[4, 0]);
}

// Original Python test: EncodingTests.test_branch_target_when_first_instruction_has_no_label
#[test]
fn branch_target_when_first_instruction_has_no_label() {
    let image = assemble("    jump -> target\ntarget::\n    return\n").unwrap();
    let offset = code_offset(&image);
    assert_eq!(&image[offset + 36 + 5..offset + 36 + 7], &[4, 1]);
}

// Original Python test: EncodingTests.test_rejects_branch_to_trailing_label_with_no_instruction
#[test]
fn rejects_branch_to_trailing_label_with_no_instruction() {
    assert_assembly_error(
        "    jump -> trailing\ntrailing::\n",
        "does not point to an instruction",
    );
}

// Original Python test: EncodingTests.test_rejects_interior_label_as_implicit_function_export
#[test]
fn rejects_interior_label_as_implicit_function_export() {
    assert_assembly_error(
        "entry::\n    nop\ninner::\n    return\nexport inner\n",
        "instruction zero",
    );
}

// Original Python test: EncodingTests.test_undefined_label_is_an_assembly_error
#[test]
fn undefined_label_is_an_assembly_error() {
    assert_assembly_error("jump -> missing\n", "undefined label");
}

// Original Python test: EncodingTests.test_rejects_unimplemented_instruction
#[test]
fn rejects_unimplemented_instruction() {
    assert_assembly_error("move.block [R1] <- [R2], R3\n", "unsupported");
}

// Original Python test: Leb128Tests.test_unsigned_boundaries_round_trip_canonically
#[test]
fn unsigned_boundaries_round_trip_canonically() {
    let boundaries: [u64; 22] = [
        0,
        1,
        127,
        128,
        255,
        16_383,
        16_384,
        (1 << 21) - 1,
        1 << 21,
        (1 << 28) - 1,
        1 << 28,
        (1 << 35) - 1,
        1 << 35,
        (1 << 42) - 1,
        1 << 42,
        (1 << 49) - 1,
        1 << 49,
        (1 << 56) - 1,
        1 << 56,
        (1 << 63) - 1,
        1 << 63,
        u64::MAX,
    ];
    for value in boundaries {
        let encoded = uleb(value);
        assert_eq!(decode_uleb(&encoded, 0).unwrap(), (value, encoded.len()));
        assert!(encoded.len() <= 10);
    }
}

// Original Python test: Leb128Tests.test_signed_boundaries_round_trip_canonically
#[test]
fn signed_boundaries_round_trip_canonically() {
    let boundaries: [i64; 23] = [
        i64::MIN,
        i64::MIN + 1,
        -(1 << 35),
        -(1 << 28),
        -(1 << 21),
        -16_384,
        -16_383,
        -129,
        -128,
        -65,
        -64,
        -63,
        -1,
        0,
        1,
        63,
        64,
        127,
        128,
        16_383,
        16_384,
        (1 << 21) - 1,
        i64::MAX,
    ];
    for value in boundaries {
        let encoded = sleb(value);
        assert_eq!(decode_sleb(&encoded, 0).unwrap(), (value, encoded.len()));
        assert!(encoded.len() <= 10);
    }
}

// Original Python test: Leb128Tests.test_unsigned_out_of_range
#[test]
fn unsigned_out_of_range() {
    assert!(u64::try_from(-1_i128).is_err());
    assert!(u64::try_from(1_i128 << 64).is_err());
}

// Original Python test: Leb128Tests.test_signed_out_of_range
#[test]
fn signed_out_of_range() {
    assert!(i64::try_from(i128::from(i64::MIN) - 1).is_err());
    assert!(i64::try_from(i128::from(i64::MAX) + 1).is_err());
}

// Original Python test: Leb128Tests.test_decoders_reject_noncanonical_and_oversized_forms
#[test]
fn decoders_reject_noncanonical_and_oversized_forms() {
    assert!(
        decode_uleb(&[0x80, 0x00], 0)
            .unwrap_err()
            .to_string()
            .contains("non-canonical")
    );
    assert!(
        decode_sleb(&[0xff, 0x7f], 0)
            .unwrap_err()
            .to_string()
            .contains("non-canonical")
    );
    assert!(
        decode_sleb(&[0x80, 0x00], 0)
            .unwrap_err()
            .to_string()
            .contains("non-canonical")
    );
    assert!(
        decode_uleb(&[0x81; 10], 0)
            .unwrap_err()
            .to_string()
            .contains("exceeds")
    );
    assert!(
        decode_sleb(&[0x81; 10], 0)
            .unwrap_err()
            .to_string()
            .contains("exceeds")
    );
    for bytes in [&[][..], &[0x80][..]] {
        assert!(
            decode_uleb(bytes, 0)
                .unwrap_err()
                .to_string()
                .contains("truncated")
        );
        assert!(
            decode_sleb(bytes, 0)
                .unwrap_err()
                .to_string()
                .contains("truncated")
        );
    }
}

// Original Python test: MalformedSourceTests.test_extra_tokens_on_line
#[test]
fn extra_tokens_on_line() {
    assert_assembly_error("nop R0\n", "expected end of line");
    assert_assembly_error("return 42\n", "expected end of line");
    assert_assembly_error("sum R0.q <- R1.q, R2.q, R3.q\n", "expected end of line");
}

// Original Python test: MalformedSourceTests.test_missing_operands_and_arrows
#[test]
fn missing_operands_and_arrows() {
    assert_assembly_error("move R0.q R1.q\n", "expected ARROW");
    assert_assembly_error("exchange R0.q <- R1.q\n", "expected SWAP");
    assert_assembly_error("jump target\n", "expected BRANCH");
    assert_assembly_error("move R0.q <-\n", "expected operand");
}

// Original Python test: MalformedSourceTests.test_invalid_destination_forms
#[test]
fn invalid_destination_forms() {
    assert_assembly_error("move 42 <- R0.q\n", "destination must be a register");
    assert_assembly_error("move [R1.q] <- R0.q\n", "destination must be a register");
    assert_assembly_error("load.u64 R0.q <- R1.q\n", "load source must be memory");
    assert_assembly_error(
        "store.u64 R0.q <- [R1.q]\n",
        "store destination must be memory",
    );
}

// Original Python test: DeterminismTests.test_repeated_assembly_is_byte_for_byte_identical
#[test]
fn repeated_assembly_is_byte_for_byte_identical() {
    let source = "unit test_det\nexport entry\nentry::\n    move R0.q <- 100\n    sum! R1.q <- R0.q, 50\n    diff R2.q <- R1.q, 25\n    and! R3.q <- R2.q, 0xFF\n    or R4.q <- R3.q, 0x100\n    xor! R5.q <- R4.q, R4.q\n    compare! R5.q, 0\n    if.zero -> done\n    store.u64 [R1.q + 16] <- R2.q\ndone::\n    return\n";
    let first = assemble(source).unwrap();
    for _ in 0..50 {
        assert_eq!(assemble(source).unwrap(), first);
    }
}

// Original Python test: GoldenByteTests.test_minimal_return_program
#[test]
fn minimal_return_program_golden() {
    let source = include_str!("golden/minimal.a");
    let image = assemble(source).unwrap();
    assert_eq!(
        image,
        decode_hex(concat!(
            "412b424301000000280000002001000000000000280000000000000005000000",
            "00000000000000000100000000000000c8000000000000001300000000000000",
            "080000000fd6366e0300000000000000e0000000000000000000000000000000",
            "08000000000000000400000000000000e0000000000000002900000000000000",
            "080000005a43c9b1050000000000000010010000000000000e00000000000000",
            "08000000e461cb6e080000000000000020010000000000000000000000000000",
            "08000000000000002b4120617373656d626c657220322e302e30000000000000",
            "0100000000000000000000002400000000000000050000000000000000000000",
            "000000000300000000000000000000000100000004006d61696e000000000000"
        ))
    );
    assert_eq!(image.len(), 288);
    assert_eq!(&image[0..4], b"A+BC");
    assert_eq!(
        (u16_at(&image, 4), u16_at(&image, 6), u16_at(&image, 8)),
        (1, 0, 40)
    );
    assert_eq!(u64_at(&image, 12), 288);
    assert_eq!(u64_at(&image, 20), 40);
    assert_eq!(u32_at(&image, 28), 5);
    assert_eq!(u32_at(&image, 32), 0);
    assert_eq!(u32_at(&image, 36), 0);
    assert_eq!(u32_at(&image, 224), 1);
    assert_eq!((u64_at(&image, 236), u64_at(&image, 244)), (36, 5));
    assert_eq!(&image[260..265], &[3, 0, 0, 0, 0]);
    assert_eq!(u32_at(&image, 272), 1);
    assert_eq!(u16_at(&image, 276), 4);
    assert_eq!(&image[278..282], b"main");
    assert_eq!(u32_at(&image, 282), 0);
}

// Original Python test: GoldenByteTests.test_canonical_source_program_bytecode_structure
#[test]
fn canonical_source_program_golden_structure() {
    let image = assemble(SOURCE).unwrap();
    assert_eq!(
        image,
        decode_hex(concat!(
            "412b424301000100280000008c01000000000000280000000000000006000000",
            "00000000000000000100000000000000e8000000000000001300000000000000",
            "080000000fd6366e030000000000000000010000000000000000000000000000",
            "0800000000000000040000000000000000010000000000005700000000000000",
            "08000000206c6ad8050000000000000058010000000000000e00000000000000",
            "0800000086237cbe080000000000000068010000000000000000000000000000",
            "0800000000000000090000000100000068010000000000002400000000000000",
            "08000000fa9c90502b4120617373656d626c657220322e302e30000000000000",
            "0100000000000000000000002400000000000000330000000000000000000000",
            "0000000010000204040060012a21000304060061006001012400020404006100",
            "605300010002040147000204050561080060030000000000010000000400626f",
            "6f74000000000000020000000000000000000000000000000000000000000000",
            "010000002e00000000000000"
        ))
    );
    let offset = code_offset(&image);
    assert_eq!(u32_at(&image, offset), 1);
    let code_length = u64_at(&image, offset + 20) as usize;
    assert_eq!(
        decode_instruction_opcodes(&image[offset..], code_length),
        [0x0010, 0x0021, 0x0024, 0x0053, 0x0047, 0x0003,]
    );
}

// Original Python test: GoldenByteTests.test_subregister_views_bytecode_encoding
#[test]
fn subregister_views_bytecode_encoding() {
    let source = include_str!("golden/views.a");
    let image = assemble(source).unwrap();
    assert_eq!(
        image,
        decode_hex(concat!(
            "412b424301000000280000005001000000000000280000000000000005000000",
            "00000000000000000100000000000000c8000000000000001300000000000000",
            "080000000fd6366e0300000000000000e0000000000000000000000000000000",
            "08000000000000000400000000000000e0000000000000005200000000000000",
            "08000000bfc3a230050000000000000038010000000000001400000000000000",
            "08000000446b396b080000000000000050010000000000000000000000000000",
            "08000000000000002b4120617373656d626c657220322e302e30000000000000",
            "01000000000000000000000024000000000000002e0000000000000000000000",
            "0000000010000201040000017f1000020205002101b4241000020308004201f8",
            "acd1910110000204040063027f0300000000000000000000010000000a007465",
            "73745f76696577730000000000000000"
        ))
    );
    let offset = code_offset(&image);
    let code = &image[offset..];
    let mut position = 36;
    let mut widths = Vec::new();
    for _ in 0..4 {
        widths.push(code[position + 3]);
        let (payload_length, payload_end) = decode_uleb(code, position + 4).unwrap();
        position = payload_end + payload_length as usize;
    }
    assert_eq!(widths, [1, 2, 3, 4]);
}

// Additional full-image compatibility check against the untouched Python package.
#[test]
#[ignore = "requires the Python reference source tree, which is not present in this checkout"]
fn python_reference_matches_rust_for_golden_sources() {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let python_source = manifest.join("../src");
    assert!(
        python_source.join("ascension_asm").is_dir(),
        "restore the Python reference under src/ to run this comparison"
    );
    let examples = [
        include_str!("golden/minimal.a"),
        include_str!("golden/views.a"),
    ];
    for source in examples {
        let mut child = Command::new("python3")
            .args(["-c", "import sys; from ascension_asm import assemble; sys.stdout.buffer.write(assemble(sys.stdin.read()))"])
            .env("PYTHONPATH", &python_source)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("python3 is required when the Python reference tree exists");
        child
            .stdin
            .take()
            .unwrap()
            .write_all(source.as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "Python reference failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout, assemble(source).unwrap());
    }
}

// Additional format boundary: reject one instruction beyond the v1 count ceiling before encoding.
#[test]
fn instruction_count_format_ceiling() {
    let instruction = Instruction {
        mnemonic: "return".into(),
        operands: Vec::new(),
        line: 1,
    };
    let program = Program {
        instructions: vec![instruction; MAX_INSTRUCTIONS + 1],
        ..Program::default()
    };
    let error: EncodeError = encode(&program).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("instruction count exceeds v1 format ceiling")
    );
}

// Additional binary boundary: parser retains exact signed 32-bit memory offset endpoints.
#[test]
fn memory_offset_signed_boundaries_encode() {
    let program =
        parse("load.u64 R0.q <- [R1.q + 0x7FFF_FFFF]\nload.u64 R0.q <- [R1.q + -0x8000_0000]\n")
            .unwrap();
    assert_eq!(program.instructions.len(), 2);
    assert!(assemble("load.u64 R0.q <- [R1.q + 0x8000_0000]\n").is_err());
    assert!(assemble("load.u64 R0.q <- [R1.q + -0x8000_0001]\n").is_err());
}

// Additional malformed API boundary: public integer encoders accept only their typed ranges.
#[test]
fn leb_integer_ranges_are_enforced_by_types() {
    let too_large_unsigned = i128::from(u64::MAX) + 1;
    let too_small_signed = i128::from(i64::MIN) - 1;
    assert!(u64::try_from(too_large_unsigned).is_err());
    assert!(i64::try_from(too_small_signed).is_err());
    assert!(u64::try_from(-1_i128).is_err());
}

#[test]
fn malformed_source_error_types_remain_distinct() {
    let error: Result<Program, ParseError> = parse("move R0.q <- @\n");
    assert!(
        error
            .unwrap_err()
            .to_string()
            .contains("unexpected character")
    );
    let assembly: Result<Vec<u8>, AssemblyError> = assemble("jump -> absent\n");
    assert!(
        assembly
            .unwrap_err()
            .to_string()
            .contains("undefined label")
    );
}

#[test]
fn prefixed_numeric_underscores_and_malformed_prefixes_match_reference_rules() {
    parse("move R0.b <- 0b_1\nmove R1.w <- 0x_FF\n").unwrap();
    assert_parse_error("move R0.q <- 0xG\n", "expected end of line");
    assert_parse_error("move R0.q <- 0b2\n", "expected end of line");
    assert_parse_error("move R0.q <- 0x_\n", "invalid number literal");
    assert_parse_error(
        "move R0.q <- 0x1_0000_0000_0000_0000_0000\n",
        "does not fit 64 bits",
    );
    assert_parse_error(
        "load.u64 R0.q <- [R1.q + 0x1_0000_0000_0000_0000_0000]\n",
        "memory offset does not fit 32 bits",
    );
}

#[test]
fn verifier_decodes_valid_non_branch_image() {
    let image = assemble(include_str!("golden/minimal.a")).unwrap();
    let verified = verify_image(&image).unwrap();
    assert_eq!(verified.entry_function, Some(0));
    assert_eq!(verified.functions.len(), 1);
    assert_eq!(verified.functions[0].instructions.len(), 1);
    assert_eq!(verified.functions[0].instructions[0].opcode, 0x0003);
}

#[test]
fn verifier_rejects_malformed_image_headers_and_sections() {
    let original = assemble(include_str!("golden/minimal.a")).unwrap();
    let mut bad_magic = original.clone();
    bad_magic[0] = b'X';
    assert!(
        verify_image(&bad_magic)
            .unwrap_err()
            .to_string()
            .contains("magic")
    );

    let mut bad_size = original.clone();
    bad_size[12] ^= 1;
    assert!(
        verify_image(&bad_size)
            .unwrap_err()
            .to_string()
            .contains("file size")
    );

    let mut bad_checksum = original.clone();
    bad_checksum[200] ^= 1;
    assert!(
        verify_image(&bad_checksum)
            .unwrap_err()
            .to_string()
            .contains("checksum")
    );

    let mut outside_directory = original;
    outside_directory[48..56].copy_from_slice(&u64::MAX.to_le_bytes());
    assert!(
        verify_image(&outside_directory)
            .unwrap_err()
            .to_string()
            .contains("section")
    );
}

#[test]
fn verifier_rejects_unknown_opcode_and_invalid_instruction_width() {
    let mut unknown = assemble(include_str!("golden/minimal.a")).unwrap();
    unknown[260..262].copy_from_slice(&0xffff_u16.to_le_bytes());
    let code_crc = ascension_asm::bytecode::crc32c(&unknown[224..265]);
    unknown[40 + 2 * 32 + 28..40 + 2 * 32 + 32].copy_from_slice(&code_crc.to_le_bytes());
    assert!(
        decode_image(&unknown)
            .unwrap_err()
            .to_string()
            .contains("unknown opcode")
    );

    let mut invalid_width = assemble(include_str!("golden/minimal.a")).unwrap();
    invalid_width[263] = 1;
    let code_crc = ascension_asm::bytecode::crc32c(&invalid_width[224..265]);
    invalid_width[40 + 2 * 32 + 28..40 + 2 * 32 + 32].copy_from_slice(&code_crc.to_le_bytes());
    assert!(
        decode_image(&invalid_width)
            .unwrap_err()
            .to_string()
            .contains("width")
    );
}

#[test]
fn avm_executes_arithmetic_and_architectural_flags() {
    let source = "export main\nmain::\n    move R0.b <- 127\n    sum! R1.b <- R0.b, 1\n    stop\n";
    let image = assemble(source).unwrap();
    let mut machine = Avm::load(&image).unwrap();
    assert_eq!(machine.run().unwrap(), RunResult::Stopped(127));
    assert_eq!(machine.registers[1], 128);
    assert_ne!(machine.flags & (1 << 2), 0);
    assert_ne!(machine.flags & (1 << 3), 0);
    assert_eq!(machine.flags & (1 << 0), 0);
    assert_eq!(machine.flags & (1 << 1), 0);
}

#[test]
fn avm_applies_narrow_write_preservation_and_dword_zero_extension() {
    let source = "export main\nmain::\n    move R0.q <- 0xFFFF_FFFF_FFFF_FFFF\n    move R0.b <- 0x12\n    move R1.d <- 0xFFFF_FFFF\n    stop\n";
    let mut machine = Avm::load(&assemble(source).unwrap()).unwrap();
    machine.run().unwrap();
    assert_eq!(machine.registers[0], 0xffff_ffff_ffff_ff12);
    assert_eq!(machine.registers[1], 0x0000_0000_ffff_ffff);
}

#[test]
fn avm_executes_exchange_and_compare_flags() {
    let source = "export main\nmain::\n    move R0.q <- 1\n    move R1.q <- 2\n    exchange R0 <-> R1\n    compare! R1.b, 2\n    stop\n";
    let mut machine = Avm::load(&assemble(source).unwrap()).unwrap();
    machine.flags = 1 << 5;
    assert_eq!(machine.run().unwrap(), RunResult::Stopped(2));
    assert_eq!(machine.registers[0], 2);
    assert_eq!(machine.registers[1], 1);
    assert_ne!(machine.flags & (1 << 1), 0);
    assert_ne!(machine.flags & (1 << 2), 0);
    assert_ne!(machine.flags & (1 << 4), 0);
    assert_ne!(machine.flags & (1 << 5), 0);
}

#[test]
fn avm_sets_carry_zero_and_even_parity_at_width_boundary() {
    let source = "export main\nmain::\n    move R0.b <- 255\n    sum! R1.b <- R0.b, 1\n    stop\n";
    let mut machine = Avm::load(&assemble(source).unwrap()).unwrap();
    machine.run().unwrap();
    assert_eq!(machine.registers[1], 0);
    assert_ne!(machine.flags & (1 << 0), 0);
    assert_ne!(machine.flags & (1 << 1), 0);
    assert_ne!(machine.flags & (1 << 4), 0);
}

#[test]
fn avm_carry_chains_and_low_width_multiply_match_isa_rules() {
    let add_source =
        "export main\nmain::\n    move R0.b <- 255\n    sumc! R1.b <- R0.b, 0\n    stop\n";
    let mut add = Avm::load(&assemble(add_source).unwrap()).unwrap();
    add.flags = 1 << 1;
    add.run().unwrap();
    assert_eq!(add.registers[1], 0);
    assert_ne!(add.flags & 1, 0);
    assert_ne!(add.flags & (1 << 1), 0);

    let sub_source =
        "export main\nmain::\n    move R0.b <- 0\n    diffc! R1.b <- R0.b, 0\n    stop\n";
    let mut sub = Avm::load(&assemble(sub_source).unwrap()).unwrap();
    sub.flags = 1 << 1;
    sub.run().unwrap();
    assert_eq!(sub.registers[1], 255);
    assert_ne!(sub.flags & (1 << 1), 0);
    assert_ne!(sub.flags & (1 << 2), 0);

    let multiply_source =
        "export main\nmain::\n    move R0.b <- 20\n    mul! R1.b <- R0.b, 13\n    stop\n";
    let mut multiply = Avm::load(&assemble(multiply_source).unwrap()).unwrap();
    multiply.run().unwrap();
    assert_eq!(multiply.registers[1], 4);
    assert_eq!(multiply.flags & ((1 << 1) | (1 << 3)), 0);
}

#[test]
fn avm_checked_sum_fault_does_not_commit_destination() {
    let source =
        "export main\nmain::\n    move R0.b <- 127\n    checked.sum R1.b <- R0.b, 1\n    stop\n";
    let mut machine = Avm::load(&assemble(source).unwrap()).unwrap();
    assert_eq!(machine.run().unwrap_err(), AvmError::IntegerOverflow);
    assert_eq!(machine.registers[1], 0);
    assert_eq!(machine.pc, 1);
}

#[test]
fn avm_unary_and_shift_rotate_operations_use_selected_width() {
    let source = "export main\nmain::\n    move R0.b <- 0x81\n    not! R5.b <- R0.b\n    shift.l! R1.b <- R0.b, 1\n    move R2.b <- 0x80\n    shift.ar R3.b <- R2.b, 1\n    rotate.r R4.b <- R0.b, 1\n    inc R6.b <- R0.b\n    dec R7.b <- R6.b\n    neg R8.b <- R0.b\n    stop\n";
    let mut machine = Avm::load(&assemble(source).unwrap()).unwrap();
    machine.run().unwrap();
    assert_eq!(machine.registers[1], 0x02);
    assert_eq!(machine.flags & (1 << 1), 1 << 1);
    assert_eq!(machine.flags & (1 << 3), 1 << 3);
    assert_eq!(machine.registers[3], 0xc0);
    assert_eq!(machine.registers[4], 0xc0);
    assert_eq!(machine.registers[5], 0x7e);
    assert_eq!(machine.registers[6], 0x82);
    assert_eq!(machine.registers[7], 0x81);
    assert_eq!(machine.registers[8], 0x7f);
}

#[test]
fn avm_branches_on_extended_architectural_predicates() {
    let source = "export main\nmain::\n    move R0.b <- 0\n    compare! R0.b, 1\n    if.sign -> selected\n    move R0.q <- 99\nselected::\n    move R0.q <- 42\n    stop\n";
    let mut machine = Avm::load(&assemble(source).unwrap()).unwrap();
    assert_eq!(machine.run().unwrap(), RunResult::Stopped(42));
}

#[test]
fn avm_executes_sign_zero_extend_and_flag_select() {
    let source = "export main\nmain::\n    move R0.b <- 0x80\n    extend.s R1.q <- R0.b\n    extend.z R2.q <- R0.b\n    compare! R0.b, 0x80\n    select R3.q <- zero, R1.q, R2.q\n    move R0.q <- R3.q\n    stop\n";
    let mut machine = Avm::load(&assemble(source).unwrap()).unwrap();
    assert_eq!(machine.run().unwrap(), RunResult::Stopped(u64::MAX - 127));
    assert_eq!(machine.registers[1], u64::MAX - 127);
    assert_eq!(machine.registers[2], 0x80);
    assert_eq!(machine.registers[3], u64::MAX - 127);
}

#[test]
fn parser_rejects_invalid_extend_widths_and_select_predicates() {
    assert_parse_error("extend.s R0.b <- R1.b\n", "narrower than destination");
    assert_parse_error("extend.z R0.w <- R1.q\n", "narrower than destination");
    assert_parse_error(
        "select R0.q <- unknown, R1.q, R2.q\n",
        "unknown flag predicate",
    );
}

#[test]
fn avm_executes_conditional_back_branch() {
    let source = "export main\nmain::\n    move R0.q <- 0\n    move R1.q <- 3\nloop::\n    diff! R1.q <- R1.q, 1\n    if.notzero -> loop\n    stop\n";
    let mut machine = Avm::load(&assemble(source).unwrap()).unwrap();
    assert_eq!(machine.run().unwrap(), RunResult::Stopped(0));
    assert_eq!(machine.registers[1], 0);
    assert_ne!(machine.flags & 1, 0);
}

#[test]
fn avm_executes_call_return_and_tailcall() {
    let call_source = "export main\nmain::\n    move R0.q <- 41\n    call -> increment\n    stop\nincrement::\n    sum R0.q <- R0.q, 1\n    return\n";
    let mut called = Avm::load(&assemble(call_source).unwrap()).unwrap();
    assert_eq!(called.run().unwrap(), RunResult::Stopped(42));

    let tail_source = "export main\nmain::\n    move R0.q <- 40\n    call -> wrapper\n    stop\nwrapper::\n    tailcall -> leaf\nleaf::\n    sum R0.q <- R0.q, 2\n    return\n";
    let mut tailed = Avm::load(&assemble(tail_source).unwrap()).unwrap();
    assert_eq!(tailed.run().unwrap(), RunResult::Stopped(42));
}

#[test]
fn avm_executes_little_endian_memory_with_region_permissions() {
    let source = "export main\nmain::\n    move R1.q <- 0x1000\n    move R0.w <- 0x1234\n    store.u16 [R1.q] <- R0.w\n    load.u16 R2.w <- [R1.q]\n    stop\n";
    let mut machine = Avm::load(&assemble(source).unwrap()).unwrap();
    machine
        .map_region(MemoryRegion {
            base: 0x1000,
            bytes: vec![0; 8],
            readable: true,
            writable: true,
            shared: false,
            atomic: false,
        })
        .unwrap();
    machine.run().unwrap();
    assert_eq!(
        &machine.memory_region(0x1000).unwrap().bytes[..2],
        &[0x34, 0x12]
    );
    assert_eq!(machine.registers[2], 0x1234);
}

#[test]
fn avm_rejects_unmapped_or_read_only_memory_access() {
    let source = "export main\nmain::\n    move R1.q <- 0x1000\n    move R0.w <- 0x1234\n    store.u16 [R1.q] <- R0.w\n    stop\n";
    let mut unmapped = Avm::load(&assemble(source).unwrap()).unwrap();
    assert!(matches!(
        unmapped.run(),
        Err(AvmError::MemoryAccess {
            address: 0x1000,
            length: 2
        })
    ));
    let mut read_only = Avm::load(&assemble(source).unwrap()).unwrap();
    read_only
        .map_region(MemoryRegion {
            base: 0x1000,
            bytes: vec![0; 8],
            readable: true,
            writable: false,
            shared: false,
            atomic: false,
        })
        .unwrap();
    assert!(matches!(
        read_only.run(),
        Err(AvmError::MemoryAccess {
            address: 0x1000,
            length: 2
        })
    ));
}

#[test]
fn avm_acquire_release_access_requires_shared_atomic_aligned_region() {
    let source = "export main\nmain::\n    move R1.q <- 0x1000\n    move R0.d <- 0x1234_5678\n    store.release.u32 [R1.q] <- R0.d\n    load.acquire.u32 R2.d <- [R1.q]\n    stop\n";
    let mut machine = Avm::load(&assemble(source).unwrap()).unwrap();
    machine
        .map_region(MemoryRegion {
            base: 0x1000,
            bytes: vec![0; 8],
            readable: true,
            writable: true,
            shared: true,
            atomic: true,
        })
        .unwrap();
    machine.run().unwrap();
    assert_eq!(machine.registers[2], 0x1234_5678);

    let misaligned = "export main\nmain::\n    move R1.q <- 0x1001\n    load.acquire.u32 R2.d <- [R1.q]\n    stop\n";
    let mut machine = Avm::load(&assemble(misaligned).unwrap()).unwrap();
    machine
        .map_region(MemoryRegion {
            base: 0x1000,
            bytes: vec![0; 8],
            readable: true,
            writable: true,
            shared: true,
            atomic: true,
        })
        .unwrap();
    assert!(matches!(
        machine.run(),
        Err(AvmError::Alignment {
            address: 0x1001,
            alignment: 4
        })
    ));

    let mut unshared = Avm::load(&assemble(source).unwrap()).unwrap();
    unshared
        .map_region(MemoryRegion {
            base: 0x1000,
            bytes: vec![0; 8],
            readable: true,
            writable: true,
            shared: false,
            atomic: true,
        })
        .unwrap();
    assert!(matches!(
        unshared.run(),
        Err(AvmError::MemoryAccess {
            address: 0x1000,
            length: 4
        })
    ));
}

#[test]
fn verifier_resolves_branch_target_to_instruction_boundary() {
    let image = assemble(include_str!("golden/canonical.a")).unwrap();
    let decoded = decode_image(&image).unwrap();
    assert_eq!(decoded.minor_version, 1);
    assert!(decoded.unresolved_control_targets.is_empty());
    assert_eq!(decoded.functions[0].blocks.get(&1), Some(&5));
    verify_image(&image).unwrap();
}

#[test]
fn verifier_rejects_block_map_offset_inside_instruction() {
    let mut image = assemble(include_str!("golden/canonical.a")).unwrap();
    let section_index = u32_at(&image, 28) as usize - 1;
    let directory_entry = 40 + section_index * 32;
    let offset = u64_at(&image, directory_entry + 8) as usize;
    let length = u64_at(&image, directory_entry + 16) as usize;
    image[offset + 4 + 16 + 8..offset + 4 + 16 + 16].copy_from_slice(&1_u64.to_le_bytes());
    let checksum = crc32c(&image[offset..offset + length]);
    image[directory_entry + 28..directory_entry + 32].copy_from_slice(&checksum.to_le_bytes());
    assert!(
        decode_image(&image)
            .unwrap_err()
            .to_string()
            .contains("instruction boundary")
    );
}

#[test]
fn deterministic_fuzz_inputs_do_not_panic_lexer_or_parser() {
    let alphabet =
        b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_!.[]<->,:;+$\r\n \t";
    let mut state = 0x7a31_59d2_u32;
    for case in 0..1500 {
        let length = (case * 37) % 192;
        let mut source = String::with_capacity(length);
        for _ in 0..length {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            source.push(alphabet[(state as usize) % alphabet.len()] as char);
        }
        let result = std::panic::catch_unwind(|| parse(&source));
        assert!(
            result.is_ok(),
            "parser panicked for generated source {source:?}"
        );
    }
}

#[test]
fn deterministic_fuzz_images_are_rejected_or_decoded_without_panics() {
    let mut state = 0xc001_d00d_u32;
    for case in 0..2000 {
        let length = (case * 53) % 384;
        let mut image = Vec::with_capacity(length);
        for _ in 0..length {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            image.push(state as u8);
        }
        let result = std::panic::catch_unwind(|| verify_image(&image));
        assert!(
            result.is_ok(),
            "verifier panicked for generated image {case}"
        );
    }
}

#[test]
fn verifier_mutation_sweep_never_panics_on_golden_image() {
    let original = assemble(include_str!("golden/canonical.a")).unwrap();
    for index in 0..original.len() {
        let mut mutated = original.clone();
        mutated[index] ^= 0x80;
        let result = std::panic::catch_unwind(|| decode_image(&mutated));
        assert!(
            result.is_ok(),
            "verifier panicked after mutation at byte {index}"
        );
    }
}

#[test]
fn cli_accepts_output_equals_and_normalizes_crlf() {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let base = std::env::temp_dir().join(format!("ascension-aa-{}-{nonce}", std::process::id()));
    let source_path = base.with_extension("a");
    let output_path = base.with_extension("avm");
    let crlf_source = include_str!("golden/minimal.a").replace('\n', "\r\n");
    std::fs::write(&source_path, crlf_source).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_aa"))
        .arg(&source_path)
        .arg(format!("--output={}", output_path.display()))
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        std::fs::read(&output_path).unwrap(),
        assemble(include_str!("golden/minimal.a")).unwrap()
    );
    std::fs::remove_file(source_path).unwrap();
    std::fs::remove_file(output_path).unwrap();
}

#[test]
fn cli_reports_assembly_errors_with_failure_status() {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let base =
        std::env::temp_dir().join(format!("ascension-aa-error-{}-{nonce}", std::process::id()));
    let source_path = base.with_extension("a");
    let output_path = base.with_extension("avm");
    std::fs::write(&source_path, "jump -> missing\n").unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_aa"))
        .arg(&source_path)
        .arg("-o")
        .arg(&output_path)
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&result.stderr).contains("undefined label"));
    assert!(!output_path.exists());
    std::fs::remove_file(source_path).unwrap();
}

#[test]
fn cli_assembles_verifies_and_executes_arithmetic_example() {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let base = std::env::temp_dir().join(format!("ascension-e2e-{}-{nonce}", std::process::id()));
    let source_path = base.with_extension("a");
    let image_path = base.with_extension("avm");
    std::fs::copy(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../examples/arithmetic.pa"),
        &source_path,
    )
    .unwrap();

    let assembled = Command::new(env!("CARGO_BIN_EXE_aa"))
        .arg(&source_path)
        .arg("-o")
        .arg(&image_path)
        .output()
        .unwrap();
    assert!(
        assembled.status.success(),
        "{}",
        String::from_utf8_lossy(&assembled.stderr)
    );

    let verified = Command::new(env!("CARGO_BIN_EXE_aa"))
        .arg("verify")
        .arg(&image_path)
        .output()
        .unwrap();
    assert!(
        verified.status.success(),
        "{}",
        String::from_utf8_lossy(&verified.stderr)
    );
    assert!(String::from_utf8_lossy(&verified.stdout).contains("verified: 1 function"));

    let executed = Command::new(env!("CARGO_BIN_EXE_aa"))
        .arg("run")
        .arg(&image_path)
        .output()
        .unwrap();
    assert!(
        executed.status.success(),
        "{}",
        String::from_utf8_lossy(&executed.stderr)
    );
    assert!(String::from_utf8_lossy(&executed.stdout).contains("stopped: 125"));

    std::fs::remove_file(source_path).unwrap();
    std::fs::remove_file(image_path).unwrap();
}

#[test]
fn cli_assembles_verifies_and_runs_every_example() {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory =
        std::env::temp_dir().join(format!("ascension-examples-{}-{nonce}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let examples = [
        (
            "hello_world",
            include_str!("../../examples/hello_world.pa"),
            0_u64,
        ),
        (
            "arithmetic",
            include_str!("../../examples/arithmetic.pa"),
            125_u64,
        ),
        (
            "registers",
            include_str!("../../examples/registers.pa"),
            100_u64,
        ),
        (
            "control_flow",
            include_str!("../../examples/control_flow.pa"),
            42_u64,
        ),
    ];
    for (name, source, expected) in examples {
        let source_path = directory.join(format!("{name}.a"));
        let image_path = directory.join(format!("{name}.avm"));
        std::fs::write(&source_path, source).unwrap();
        let assembled = Command::new(env!("CARGO_BIN_EXE_aa"))
            .arg(&source_path)
            .arg("-o")
            .arg(&image_path)
            .output()
            .unwrap();
        assert!(
            assembled.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&assembled.stderr)
        );

        let verified = Command::new(env!("CARGO_BIN_EXE_aa"))
            .arg("verify")
            .arg(&image_path)
            .output()
            .unwrap();
        assert!(
            verified.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&verified.stderr)
        );

        let executed = Command::new(env!("CARGO_BIN_EXE_aa"))
            .arg("run")
            .arg(&image_path)
            .output()
            .unwrap();
        assert!(
            executed.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&executed.stderr)
        );
        assert!(
            String::from_utf8_lossy(&executed.stdout).contains(&format!("stopped: {expected}")),
            "{name}: {}",
            String::from_utf8_lossy(&executed.stdout)
        );
    }
    std::fs::remove_dir_all(directory).unwrap();
}

#[cfg(all(target_arch = "x86_64", target_os = "linux"))]
#[test]
fn native_aot_compiles_and_executes_supported_arithmetic_program() {
    let source = "export main\nmain::\n    move R0.q <- 40\n    sum! R1.q <- R0.q, 2\n    move R0.q <- R1.q\n    stop\n";
    let image = assemble(source).unwrap();
    let mut avm = Avm::load(&image).unwrap();
    assert_eq!(avm.run().unwrap(), RunResult::Stopped(42));

    let assembly = emit_assembly(&image).unwrap();
    assert!(assembly.contains("addq"));
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let executable =
        std::env::temp_dir().join(format!("ascension-aot-{}-{nonce}", std::process::id()));
    compile_executable(&image, &executable).unwrap();
    let result = Command::new(&executable).status().unwrap();
    assert_eq!(result.code(), Some(42));
    std::fs::remove_file(executable).unwrap();
}

#[cfg(all(target_arch = "x86_64", target_os = "linux"))]
#[test]
fn native_aot_lowers_flag_branch_call_and_exchange() {
    let image = assemble(include_str!("../../examples/control_flow.pa")).unwrap();
    let mut avm = Avm::load(&image).unwrap();
    assert_eq!(avm.run().unwrap(), RunResult::Stopped(42));
    assert_eq!(run_jit(&image).unwrap(), 42);

    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let executable =
        std::env::temp_dir().join(format!("ascension-aot-flow-{}-{nonce}", std::process::id()));
    compile_executable(&image, &executable).unwrap();
    let result = Command::new(&executable).status().unwrap();
    assert_eq!(result.code(), Some(42));
    std::fs::remove_file(executable).unwrap();
}

#[cfg(all(target_arch = "x86_64", target_os = "linux"))]
#[test]
fn cli_compile_native_emits_runnable_executable() {
    let image = assemble(include_str!("../../examples/arithmetic.pa")).unwrap();
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let base =
        std::env::temp_dir().join(format!("ascension-aot-cli-{}-{nonce}", std::process::id()));
    let image_path = base.with_extension("avm");
    let executable = base.with_extension("bin");
    std::fs::write(&image_path, image).unwrap();
    let compiled = Command::new(env!("CARGO_BIN_EXE_aa"))
        .arg("compile-native")
        .arg(&image_path)
        .arg("-o")
        .arg(&executable)
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    assert_eq!(
        Command::new(&executable).status().unwrap().code(),
        Some(125)
    );
    std::fs::remove_file(image_path).unwrap();
    std::fs::remove_file(executable).unwrap();
}

#[cfg(all(target_arch = "x86_64", target_os = "linux"))]
#[test]
fn jit_executes_loaded_native_code_and_matches_avm() {
    let source = "export main\nmain::\n    move R0.q <- 2\nloop::\n    sum! R0.q <- R0.q, 1\n    compare! R0.q, 5\n    if.notzero -> loop\n    stop\n";
    let image = assemble(source).unwrap();
    let mut avm = Avm::load(&image).unwrap();
    assert_eq!(avm.run().unwrap(), RunResult::Stopped(5));
    assert_eq!(run_jit(&image).unwrap(), 5);
}

#[cfg(all(target_arch = "x86_64", target_os = "linux"))]
#[test]
fn cli_run_jit_executes_supported_image() {
    let image = assemble(include_str!("../../examples/arithmetic.pa")).unwrap();
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let image_path = std::env::temp_dir().join(format!(
        "ascension-jit-cli-{}-{nonce}.avm",
        std::process::id()
    ));
    std::fs::write(&image_path, image).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_aa"))
        .arg("run-jit")
        .arg(&image_path)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(String::from_utf8_lossy(&result.stdout).contains("jit stopped: 125"));
    std::fs::remove_file(image_path).unwrap();
}

/// CRLF (and lone CR) line endings assemble identically through the library
/// API, not only through the CLI which normalises them first.
#[test]
fn crlf_source_assembles_via_library() {
    let lf = "export main\nmain::\n    move R0.q <- 42\n    stop\n";
    let crlf = lf.replace('\n', "\r\n");
    let cr = lf.replace('\n', "\r");
    let expected = assemble(lf).unwrap();
    assert_eq!(assemble(&crlf).unwrap(), expected);
    assert_eq!(assemble(&cr).unwrap(), expected);
}
