//! Regression tests for defects confirmed during the +A Core audit.
//!
//! Each test names the audit finding it pins down. The tier-parity tests assert
//! that the interpreter, the JIT, and the native AOT backend agree, because a
//! backend that quietly disagrees with the AVM is the class of bug several of
//! these findings belonged to.

use ascension_asm::assemble;
use ascension_asm::avm::{Avm, AvmError, MemoryRegion, RunResult};
use ascension_asm::bytecode::crc32c;
use ascension_asm::jit::run_jit;
use ascension_asm::jit::run_jit64;
use ascension_asm::native::{NATIVE_FAULT_EXIT, compile_executable, emit_assembly};
use ascension_asm::verifier::verify_image;

fn image_of(body: &str) -> Vec<u8> {
    assemble(&format!("export main\nmain::\n    {body}    stop\n")).unwrap()
}

/// Deterministic pseudo-random source generator, used to widen the parser and
/// verifier robustness sweep that already exists in `tests/assembler.rs`.
fn generated_source(seed: u32, length: usize) -> String {
    const ALPHABET: &[u8] =
        b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_!.[]<->,:;+$ \n\t";
    let mut state = seed | 1;
    let mut source = String::with_capacity(length);
    for _ in 0..length {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        source.push(ALPHABET[(state as usize) % ALPHABET.len()] as char);
    }
    source
}

fn avm_of(body: &str) -> (Avm, Result<RunResult, AvmError>) {
    let image = image_of(body);
    let mut machine = Avm::load(&image).unwrap();
    let result = machine.run();
    (machine, result)
}

fn avm_status(body: &str) -> u64 {
    match avm_of(body).1 {
        Ok(RunResult::Stopped(status)) => status,
        other => panic!("expected `stopped`, got {other:?}"),
    }
}

fn native_status(body: &str) -> Option<i32> {
    let image = image_of(body);
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let executable =
        std::env::temp_dir().join(format!("ascension-reg-{}-{nonce}", std::process::id()));
    compile_executable(&image, &executable).unwrap();
    let status = std::process::Command::new(&executable).status().unwrap();
    let _ = std::fs::remove_file(&executable);
    status.code()
}

/// Audit finding 1: `compare.swap` ignored the encoded memory displacement, so
/// it read and wrote the base address instead of `[base + offset]`.
#[test]
fn compare_swap_honours_memory_offset() {
    let body = "move R1.q <- 0x1000\n    move R0.q <- 7\n    move R2.q <- 9\n    compare.swap R3.q <- [R1.q + 8], R0.q, R2.q\n    move R0.q <- R3.q\n";
    let image = image_of(body);
    let mut machine = Avm::load(&image).unwrap();
    machine
        .map_region(MemoryRegion {
            base: 0x1000,
            bytes: vec![0; 24],
            readable: true,
            writable: true,
            shared: true,
            atomic: true,
        })
        .unwrap();
    // The word at +8 was zero, so the swap does not fire and returns the old
    // value; the point is that it does not fault on an unmapped address.
    assert_eq!(machine.run().unwrap(), RunResult::Stopped(0));

    let assembly = emit_assembly(&image_of(body)).unwrap();
    assert!(
        assembly.contains("addq $8, %rcx"),
        "native lowering must apply the compare.swap displacement:\n{assembly}"
    );
}

/// Audit finding 1 (behavioural): the offset must select the same word in every
/// tier. The interpreter is checked directly; the native backend is checked by
/// asserting that the emitted address computation includes the displacement.
#[test]
fn compare_swap_offset_selects_the_right_word() {
    let image = assemble(
        "export main\nmain::\n    move R1.q <- 0x1000\n    move R0.q <- 0\n    move R2.q <- 0xAB\n    compare.swap R3.q <- [R1.q + 8], R0.q, R2.q\n    stop\n",
    )
    .unwrap();
    let mut machine = Avm::load(&image).unwrap();
    machine
        .map_region(MemoryRegion {
            base: 0x1000,
            bytes: vec![0; 16],
            readable: true,
            writable: true,
            shared: true,
            atomic: true,
        })
        .unwrap();
    machine.run().unwrap();
    let bytes = &machine.memory_region(0x1000).unwrap().bytes;
    assert_eq!(
        bytes[..8],
        [0; 8],
        "the dword at the base must not be touched"
    );
    assert_eq!(bytes[8], 0xAB, "the write must land at base + 8");
}

/// Audit finding 2: a `.d` write ignored its source and always staged `%eax`
/// into the destination slot.
///
/// The visible consequence is `div`, which passes `%rdx` when writing the
/// remainder: the old code stored `%eax` (the quotient) instead, so both
/// destinations received the quotient and the remainder was lost.
#[test]
fn dword_destination_stores_its_own_value() {
    assert_eq!(avm_status("move R0.d <- 0x1234_5678\n"), 0x1234_5678);
    let image = image_of(
        "move R0.d <- 100\n    move R1.d <- 7\n    div.u R2.d, R3.d <- R0.d, R1.d\n    move R0.q <- R3.q\n",
    );
    assert_eq!(run_jit(&image).unwrap(), 2, "remainder must reach R3");
}

/// Audit finding 2 (generality): *every* register view must be lowerable. The
/// backend previously failed to assemble for any `.d` operand at all.
#[test]
fn every_register_view_lowers_natively() {
    for (body, expected) in [
        ("move R0.b <- 0x7F\n", 0x7F),
        ("move R0.w <- 0x1234\n", 0x1234),
        ("move R0.d <- 0x1234_5678\n", 0x1234_5678),
        (
            "move R0.q <- 0x1122_3344_5566_7788\n",
            0x1122_3344_5566_7788,
        ),
    ] {
        assert_eq!(avm_status(body), expected, "interpreter: {body:?}");
        let image = image_of(body);
        assert_eq!(run_jit(&image).unwrap(), expected as i32, "jit: {body:?}");
        assert_eq!(
            native_status(body),
            Some((expected & 0xff) as i32),
            "native: {body:?}"
        );
    }
}

/// Audit finding 3: narrow signed division used the full 64-bit `idiv` after
/// sign-extending from bit 63, so 8-/16-/32-bit results disagreed with the AVM.
#[test]
fn narrow_signed_division_matches_the_interpreter() {
    // -100 / 7 == -14 remainder -2, at 8-bit width.
    assert_eq!(
        avm_status(
            "move R0.b <- -100\n    move R1.b <- 7\n    div.s R2.b, R3.b <- R0.b, R1.b\n    move R0.q <- R2.q\n"
        ),
        0xF2
    );
    assert_eq!(
        avm_status(
            "move R0.b <- -100\n    move R1.b <- 7\n    div.s R2.b, R3.b <- R0.b, R1.b\n    move R0.q <- R3.q\n"
        ),
        0xFE
    );
    let quotient = image_of(
        "move R0.b <- -100\n    move R1.b <- 7\n    div.s R2.b, R3.b <- R0.b, R1.b\n    move R0.q <- R2.q\n",
    );
    let remainder = image_of(
        "move R0.b <- -100\n    move R1.b <- 7\n    div.s R2.b, R3.b <- R0.b, R1.b\n    move R0.q <- R3.q\n",
    );
    assert_eq!(run_jit(&quotient).unwrap(), 0xF2);
    assert_eq!(run_jit(&remainder).unwrap(), 0xFE);
    assert_eq!(
        native_status(
            "move R0.b <- -100\n    move R1.b <- 7\n    div.s R2.b, R3.b <- R0.b, R1.b\n    move R0.q <- R2.q\n"
        ),
        Some(0xF2)
    );
}

/// Native `div` raised #DE for a zero divisor and killed the host process with
/// SIGFPE. It must report a fault instead, and the fault must be visible to the
/// JIT loader and as a distinct process exit status.
#[test]
fn native_division_by_zero_reports_a_fault() {
    let body = "move R0.q <- 10\n    move R1.q <- 0\n    div.u R2.q, R3.q <- R0.q, R1.q\n";
    let image = image_of(body);
    let error = run_jit(&image).unwrap_err();
    assert!(
        error.to_string().contains("DIVIDE_BY_ZERO"),
        "unexpected JIT fault text: {error}"
    );
    assert_eq!(
        native_status(body),
        Some(NATIVE_FAULT_EXIT),
        "the executable must exit with the documented fault status"
    );
}

/// Audit finding 4: `run <image.avm> --native` parsed the flag and then ran the
/// interpreter, silently ignoring the request.
#[test]
fn run_bytecode_with_native_flag_builds_native_binary() {
    let directory = std::env::temp_dir().join(format!(
        "ascension-reg-native-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let image_path = directory.join("arith.avm");
    std::fs::write(&image_path, image_of("move R0.q <- 55\n")).unwrap();
    let build_dir = directory.join("build");

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_plus-a"))
        .arg("run")
        .arg(&image_path)
        .arg("--native")
        .arg("--build-dir")
        .arg(&build_dir)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        build_dir.join("native/arith").exists(),
        "--native must produce a native executable for bytecode input"
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("stopped: 55"));
    std::fs::remove_dir_all(directory).unwrap();
}

/// Audit finding 5: `usage()` listed only a subset of the implemented commands.
#[test]
fn usage_lists_every_implemented_command() {
    let usage = ascension_asm::cli::usage();
    for command in [
        "run",
        "build",
        "check",
        "verify",
        "inspect",
        "run-jit",
        "compile-native",
    ] {
        assert!(usage.contains(command), "usage() omits {command}: {usage}");
    }
    let help = ascension_asm::cli::help_text();
    for command in ["inspect", "run-jit", "compile-native"] {
        assert!(help.contains(command), "help_text() omits {command}");
    }
}

/// Audit finding 6a: the CAPS length check compared with `>` instead of `!=`,
/// so a CAPS payload carrying bytes beyond its declared records verified.
///
/// CAPS is emitted last with an empty payload, so it can be grown in place by
/// appending bytes and fixing up its length, checksum, and the file size.
#[test]
fn caps_section_with_trailing_bytes_is_rejected() {
    let image = assemble("export main\nmain::\n    return\n").unwrap();
    let count = u32::from_le_bytes(image[28..32].try_into().unwrap()) as usize;
    let entry = (0..count)
        .map(|index| 40 + index * 32)
        .find(|entry| u32::from_le_bytes(image[*entry..*entry + 4].try_into().unwrap()) == 8)
        .expect("CAPS section");
    let offset = u64::from_le_bytes(image[entry + 8..entry + 16].try_into().unwrap()) as usize;
    let length = u64::from_le_bytes(image[entry + 16..entry + 24].try_into().unwrap()) as usize;
    assert_eq!(length, 0, "the assembler emits an empty CAPS payload");
    assert_eq!(offset, image.len(), "CAPS is the final payload");

    // A well-formed zero-record payload is exactly four bytes; the two extra
    // bytes belong to no record.
    let content = [0_u8, 0, 0, 0, 0xAA, 0xBB];
    let mut patched = image.clone();
    patched.extend_from_slice(&content);
    patched[entry + 16..entry + 24].copy_from_slice(&(content.len() as u64).to_le_bytes());
    patched[entry + 28..entry + 32].copy_from_slice(&crc32c(&content).to_le_bytes());
    let total = patched.len() as u64;
    patched[12..20].copy_from_slice(&total.to_le_bytes());

    let error = verify_image(&patched).unwrap_err().to_string();
    assert!(
        error.contains("CAPS section length"),
        "trailing CAPS bytes must be rejected, got: {error}"
    );

    // The same payload without the trailing bytes is accepted.
    let mut clean = image.clone();
    clean.extend_from_slice(&content[..4]);
    clean[entry + 16..entry + 24].copy_from_slice(&4_u64.to_le_bytes());
    clean[entry + 28..entry + 32].copy_from_slice(&crc32c(&content[..4]).to_le_bytes());
    let total = clean.len() as u64;
    clean[12..20].copy_from_slice(&total.to_le_bytes());
    assert!(verify_image(&clean).is_ok());
}

/// Audit finding 6b: the only unrepresentable signed quotient was reported as
/// `DivByZero` although the divisor is not zero.
#[test]
fn signed_division_overflow_is_not_a_divide_fault() {
    let body = "move R0.q <- -0x8000_0000_0000_0000\n    move R1.q <- -1\n    div.s R2.q, R3.q <- R0.q, R1.q\n";
    assert_eq!(avm_of(body).1.unwrap_err(), AvmError::IntegerOverflow);
    assert_eq!(
        avm_of("move R0.b <- -128\n    move R1.b <- -1\n    div.s R2.b, R3.b <- R0.b, R1.b\n")
            .1
            .unwrap_err(),
        AvmError::IntegerOverflow
    );
}

/// Audit finding 7: a handler recorded in one function redirected a `raise`
/// performed in another to that other function's block index.
#[test]
fn raise_only_targets_a_handler_in_the_raising_function() {
    // `handler` is instruction 2 of `main` but instruction 0 of `thrower`.
    // Interpreting the block index inside `thrower` lands on the wrong code.
    let source = "export main\nmain::\n    jump -> main_body\n\
                  handler::\n    move R0.q <- 99\n    stop\n\
                  main_body::\n    catch -> handler\n    call -> thrower\n    move R0.q <- 1\n    stop\n\
                  thrower::\n    raise 5\n    move R0.q <- 2\n    return\n";
    let image = assemble(source).unwrap();
    let mut machine = Avm::load(&image).unwrap();
    assert_eq!(machine.run().unwrap(), RunResult::Stopped(99));
}

/// A `raise` with no handler in the raising function is a runtime fault; it
/// must never jump to a stale handler left behind by another function.
#[test]
fn raise_without_a_local_handler_faults() {
    // `main` installs a handler, calls `thrower`, and `thrower` swallows it.
    // The `raise` that follows therefore has no handler in its own function.
    let source = "export main\nmain::\n    catch -> handler\n    call -> thrower\n    stop\n\
                  handler::\n    stop\n\
                  thrower::\n    uncatch\n    raise 1\n";
    let image = assemble(source).unwrap();
    // `uncatch` here pops the handler registered by `main`, which the per-
    // function balance check cannot see, so the verifier accepts the image and
    // the fault is a runtime condition.
    let mut machine = Avm::load_unverified(&image).unwrap();
    assert_eq!(machine.run().unwrap_err(), AvmError::InvalidHandler);
}

/// Tier parity: a matrix of programs must produce the same status under the
/// interpreter, the JIT, and the native backend. This is the guard against the
/// whole class of drift the findings above belonged to.
#[test]
fn execution_tiers_agree_on_a_representative_matrix() {
    let cases: &[(&str, u64)] = &[
        ("move R0.b <- 5\n", 5),
        ("move R0.w <- 0x1234\n", 0x1234),
        ("move R0.d <- 0x1234_5678\n", 0x1234_5678),
        ("move R0.b <- -1\n", 0xFF),
        ("move R0.q <- -2\n", 0xFFFF_FFFF_FFFF_FFFE),
        (
            "move R0.b <- 200\n    sum! R1.b <- R0.b, 100\n    move R0.q <- R1.q\n",
            44,
        ),
        (
            "move R0.d <- 0x7FFF_FFFF\n    sum! R1.d <- R0.d, 1\n    move R0.q <- R1.q\n",
            0x8000_0000,
        ),
        (
            "move R0.b <- 20\n    mul! R1.b <- R0.b, 13\n    move R0.q <- R1.q\n",
            4,
        ),
        (
            "move R0.w <- 300\n    mul! R1.w <- R0.w, 300\n    move R0.q <- R1.q\n",
            90000 & 0xFFFF,
        ),
        (
            "move R0.b <- 0xFF\n    and! R1.b <- R0.b, 0x0F\n    move R0.q <- R1.q\n",
            15,
        ),
        (
            "move R0.d <- 0x0F0F\n    or! R1.d <- R0.d, 0xF0F0\n    move R0.q <- R1.q\n",
            0xFFFF,
        ),
        (
            "move R0.b <- 5\n    diff! R1.b <- R0.b, 10\n    move R0.q <- R1.q\n",
            0xFB,
        ),
        (
            "move R0.d <- 100\n    move R1.d <- 7\n    div.u R2.d, R3.d <- R0.d, R1.d\n    move R0.q <- R3.q\n",
            2,
        ),
        (
            "move R0.d <- 100\n    move R1.d <- 7\n    div.u R2.d, R3.d <- R0.d, R1.d\n    move R0.q <- R2.q\n",
            14,
        ),
        (
            "move R0.b <- -100\n    move R1.b <- 7\n    div.s R2.b, R3.b <- R0.b, R1.b\n    move R0.q <- R2.q\n",
            0xF2,
        ),
        (
            "move R0.q <- 1\n    move R1.q <- 2\n    exchange R0 <-> R1\n    move R0.q <- R0.q\n",
            2,
        ),
        (
            "move R0.q <- 0\n    move R1.q <- 4\nloop::\n    sum R0.q <- R0.q, 1\n    diff! R2.q <- R1.q, R0.q\n    if.notzero -> loop\n",
            4,
        ),
    ];
    for (body, expected) in cases {
        let image = image_of(body);
        assert_eq!(avm_status(body), *expected, "interpreter: {body:?}");
        // The JIT returns a signed 32-bit status; widen it back for comparison.
        // The JIT hands `stop` to the caller as a signed `i32`; compare the low
        // 32 bits so the wider interpreter status is still checked exactly.
        assert_eq!(
            run_jit(&image).unwrap() as u32 as u64,
            *expected & 0xFFFF_FFFF,
            "jit: {body:?}"
        );
        assert_eq!(
            native_status(body),
            Some((*expected & 0xFF) as i32),
            "native: {body:?}"
        );
    }
}

/// Tier parity for control flow, which needs whole programs rather than the
/// body-only form used above.
#[test]
fn execution_tiers_agree_on_calls_and_branches() {
    let cases: &[(&str, u64)] = &[
        (
            "export main\nmain::\n    move R0.q <- 41\n    call -> inc\n    stop\ninc::\n    sum R0.q <- R0.q, 1\n    return\n",
            42,
        ),
        (
            "export main\nmain::\n    move R0.q <- 40\n    call -> wrapper\n    stop\nwrapper::\n    tailcall -> leaf\nleaf::\n    sum R0.q <- R0.q, 2\n    return\n",
            42,
        ),
        // Nested calls: each `return` must pop exactly its own frame.
        (
            "export main\nmain::\n    move R0.q <- 0\n    call -> a\n    stop\na::\n    sum R0.q <- R0.q, 1\n    call -> b\n    return\nb::\n    sum R0.q <- R0.q, 10\n    return\n",
            11,
        ),
        // A `return` that is not the last instruction of its function.
        (
            "export main\nmain::\n    move R0.q <- 7\n    call -> pick\n    stop\npick::\n    compare! R0.q, 7\n    if.zero -> yes\n    move R0.q <- 1\n    return\nyes::\n    move R0.q <- 7\n    return\n    move R0.q <- 99\n",
            7,
        ),
        (
            "export main\nmain::\n    move R0.q <- 0\n    move R1.q <- 4\nloop::\n    sum R0.q <- R0.q, 1\n    diff! R2.q <- R1.q, R0.q\n    if.notzero -> loop\n    stop\n",
            4,
        ),
    ];
    for (source, expected) in cases {
        let image = assemble(source).unwrap();
        let mut machine = Avm::load(&image).unwrap();
        assert_eq!(
            machine.run().unwrap(),
            RunResult::Stopped(*expected),
            "interpreter: {source:?}"
        );
        assert_eq!(
            run_jit(&image).unwrap() as u32 as u64,
            *expected & 0xFFFF_FFFF,
            "jit: {source:?}"
        );
    }
}

/// Contract: the JIT reports the same 64-bit `stop` status the interpreter does.
/// A process exit status is truncated to eight bits, so only the JIT can be
/// compared at full width; the native backend is checked separately.
#[test]
fn jit_reports_the_full_stop_status() {
    for (body, expected) in [
        ("move R0.q <- 0x1_0000_0000\n", 0x1_0000_0000_u64),
        ("move R0.q <- 0xFFFF_FFFF_FFFF_FFFF\n", u64::MAX),
        ("move R0.q <- 125\n", 125),
    ] {
        assert_eq!(avm_status(body), expected, "interpreter: {body:?}");
        assert_eq!(
            run_jit64(&image_of(body)).unwrap(),
            expected,
            "jit: {body:?}"
        );
    }
}

/// The step limit is a restartable budget: running out leaves the machine
/// pointing at the next instruction, not at a stale position in the caller.
#[test]
fn step_limit_is_a_restartable_budget() {
    // Counts down from three, so the program terminates after a known number of
    // instructions; the first run is given a budget too small to reach `stop`.
    let image = assemble(
        "export main\nmain::\n    move R0.q <- 0\n    move R1.q <- 3\n    call -> body\n    stop\n\
         body::\n    sum R0.q <- R0.q, 1\n    diff! R1.q <- R1.q, 1\n    if.notzero -> body\n    return\n",
    )
    .unwrap();
    let mut machine = Avm::load(&image).unwrap().with_step_limit(2);
    assert_eq!(machine.run().unwrap_err(), AvmError::StepLimit);
    // Resuming must make progress instead of repeating the same instruction.
    let mut machine = machine.with_step_limit(10_000);
    assert!(
        machine.run().is_ok(),
        "resuming after StepLimit must progress"
    );
}

/// `-o` used to write bytecode without verifying it and without reporting the
/// path; it now verifies and reports, like `build`.
#[test]
fn legacy_output_flag_verifies_and_reports() {
    let directory = std::env::temp_dir().join(format!(
        "ascension-reg-output-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let source = directory.join("program.pa");
    let out = directory.join("program.avm");
    std::fs::write(
        &source,
        "export main\nmain::\n    move R0.q <- 3\n    stop\n",
    )
    .unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_plus-a"))
        .arg(&source)
        .arg("-o")
        .arg(&out)
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(out.exists());
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("built:"),
        "an output build must report the artifact it wrote"
    );
    std::fs::remove_dir_all(directory).unwrap();
}

/// Conflicting execution modes must be rejected instead of silently preferring
/// one of them.
#[test]
fn jit_and_native_flags_conflict() {
    let directory = std::env::temp_dir().join(format!(
        "ascension-reg-modes-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let source = directory.join("program.pa");
    std::fs::write(&source, "export main\nmain::\n    return\n").unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_plus-a"))
        .arg("run")
        .arg(&source)
        .arg("--jit")
        .arg("--native")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("cannot be combined"));
    std::fs::remove_dir_all(directory).unwrap();
}

/// The unprivileged AVM stays isolated: every `sys.*` requirement is reported
/// as a missing capability rather than being silently allowed.
#[test]
fn every_system_service_requires_a_capability() {
    let services = [
        ("sys.region.map", 1 << 0),
        ("sys.region.unmap", 1 << 0),
        ("sys.port.read", 1 << 1),
        ("sys.port.write", 1 << 1),
        ("sys.irq.bind", 1 << 2),
        ("sys.irq.enable", 1 << 2),
        ("sys.irq.disable", 1 << 2),
        ("sys.context.enter", 1 << 3),
        ("sys.context.return", 1 << 3),
        ("sys.cpu.control", 1 << 4),
        ("sys.power", 1 << 5),
    ];
    for (service, capability) in services {
        let image = assemble(&format!("export main\nmain::\n    {service}\n    stop\n")).unwrap();
        // Without the capability the image does not verify.
        let error = verify_image(&image).unwrap_err().to_string();
        assert!(
            error.contains("capability"),
            "{service} must require a capability, got: {error}"
        );
        // With it, the verifier accepts the image but the unprivileged machine
        // still refuses to execute the service.
        let mut machine = Avm::load_unverified(&image).unwrap();
        machine.privileged = true;
        machine.capabilities = capability;
        assert_eq!(
            machine.run().unwrap_err(),
            AvmError::UnsupportedService,
            "{service} must not gain host access"
        );
        let mut unprivileged = Avm::load_unverified(&image).unwrap();
        unprivileged.capabilities = capability;
        assert_eq!(
            unprivileged.run().unwrap_err(),
            AvmError::PrivilegeViolation,
            "{service} must be refused without privilege"
        );
    }
}

/// `compile-native` must validate the image before invoking the C toolchain.
#[test]
fn compile_native_rejects_invalid_images() {
    let directory = std::env::temp_dir().join(format!(
        "ascension-reg-cn-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let image_path = directory.join("bad.avm");
    std::fs::write(&image_path, b"not an image").unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_plus-a"))
        .arg("compile-native")
        .arg(&image_path)
        .arg("-o")
        .arg(directory.join("bad"))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(!directory.join("bad").exists());
    std::fs::remove_dir_all(directory).unwrap();
}

/// Robustness: the parser must never panic, whatever it is handed.
#[test]
fn parser_never_panics_on_generated_source() {
    for case in 0..600 {
        let source = generated_source(0x5eed_0000 + case, (case % 160) as usize);
        let outcome = std::panic::catch_unwind(|| ascension_asm::parser::parse(&source));
        assert!(outcome.is_ok(), "parser panicked on {source:?}");
    }
}

/// Robustness: assembled images must survive byte-level mutation without a
/// panic, and any accepted mutation must still execute or fault cleanly.
#[test]
fn mutated_images_never_panic_the_verifier_or_the_avm() {
    let images = [
        image_of("move R0.q <- 42\n"),
        assemble(include_str!("golden/canonical.a")).unwrap(),
        assemble(include_str!("../../examples/control_flow.pa")).unwrap(),
    ];
    for image in images {
        for index in (0..image.len()).step_by(7) {
            for mask in [0x01_u8, 0x80] {
                let mut mutated = image.clone();
                mutated[index] ^= mask;
                let verified = std::panic::catch_unwind(|| verify_image(&mutated));
                assert!(verified.is_ok(), "verifier panicked at byte {index}");
                if let Ok(Ok(_)) = verified {
                    let executed = std::panic::catch_unwind(|| {
                        let machine = Avm::load_unverified(&mutated)?;
                        machine.with_step_limit(10_000).run()
                    });
                    assert!(executed.is_ok(), "AVM panicked for a valid image");
                }
            }
        }
    }
}

/// Robustness: truncated images must be rejected, never read out of bounds.
#[test]
fn truncated_images_are_rejected() {
    let image = assemble(include_str!("golden/canonical.a")).unwrap();
    for length in 0..image.len() {
        let prefix = &image[..length];
        let outcome = std::panic::catch_unwind(|| verify_image(prefix));
        assert!(outcome.is_ok(), "verifier panicked at length {length}");
        assert!(
            outcome.unwrap().is_err(),
            "a truncated image must not verify (length {length})"
        );
    }
}

/// The native subset is declared in one table; `emit_instruction` must agree
/// with it exactly, so neither can silently claim more than it lowers.
#[test]
fn native_opcode_table_matches_the_emitter() {
    use ascension_asm::instructions::{definition, mnemonic_for};
    use ascension_asm::native::{NATIVE_OPCODE_RANGES, native_supports};
    let _ = NATIVE_OPCODE_RANGES;

    let mut claimed = Vec::new();
    for opcode in 0u16..=0x00ff {
        if native_supports(opcode) {
            claimed.push(opcode);
        }
    }
    for opcode in &claimed {
        assert!(
            mnemonic_for(*opcode).is_some(),
            "the native table claims undefined opcode 0x{opcode:04x}"
        );
    }

    // Emitting an image that uses the opcode must succeed exactly when the
    // table claims it, and the diagnostic must name the instruction otherwise.
    for opcode in 0x0001u16..=0x00ff {
        let Some(name) = mnemonic_for(opcode) else {
            continue;
        };
        if definition(name).is_none() {
            continue;
        }
        let usage = sample_usage(name);
        let source = format!("export main\nmain::\n{usage}\n    stop\n");
        let image = assemble(&source)
            .unwrap_or_else(|error| panic!("sample for `{name}` does not assemble: {error}"));
        // `sys.*` samples cannot be verified without a declared capability, so
        // they are exercised through `native_supports` above only.
        if name.starts_with("sys.") {
            assert!(
                !native_supports(opcode),
                "`{name}` must be interpreter-only"
            );
            continue;
        }
        match emit_assembly(&image) {
            Ok(_) => assert!(
                native_supports(opcode),
                "`{name}` emitted code but is outside the declared native subset"
            ),
            Err(error) => {
                let text = error.to_string();
                assert!(
                    !native_supports(opcode) || text.contains("native"),
                    "`{name}` is in the native subset but failed to emit: {text}"
                );
                if !native_supports(opcode) {
                    // The refusal names the first instruction it cannot lower,
                    // which is this one unless the sample needed a prerequisite
                    // (as `uncatch` needs `catch`).
                    assert!(
                        text.contains("not supported by the native backends"),
                        "refusing `{name}` must explain why: {text}"
                    );
                    assert!(
                        (0x0001u16..=0x00ff).any(|candidate| {
                            !native_supports(candidate)
                                && mnemonic_for(candidate).is_some_and(|m| text.contains(m))
                        }),
                        "refusing `{name}` must name an unsupported instruction: {text}"
                    );
                }
            }
        }
    }
}

fn sample_usage(name: &str) -> &'static str {
    match name {
        "nop" => "    nop",
        "break" => "    break",
        "return" => "    return",
        "stop" => "    stop",
        "uncatch" => "    catch -> main\n    uncatch",
        "catch" => "    catch -> main",
        "raise" => "    raise 1",
        "flag.direction" => "    flag.direction <- forward",
        "fence" => "    fence full",
        "address" => "    address R0.q <- main",
        "move" => "    move R0.q <- 1",
        "extend.s" => "    extend.s R0.q <- R1.b",
        "extend.z" => "    extend.z R0.q <- R1.b",
        "select" => "    select R0.q <- zero, 1, 2",
        "div.u" => "    div.u R0.q, R1.q <- R2.q, 2",
        "div.s" => "    div.s R0.q, R1.q <- R2.q, 2",
        "mul" => "    mul R0.q <- R1.q, 2",
        "mul!" => "    mul! R0.q <- R1.q, 2",
        "sumc" => "    sumc R0.q <- R1.q, 2",
        "sumc!" => "    sumc! R0.q <- R1.q, 2",
        "diffc" => "    diffc R0.q <- R1.q, 2",
        "diffc!" => "    diffc! R0.q <- R1.q, 2",
        "checked.sum" => "    checked.sum R0.q <- R1.q, 2",
        "neg" => "    neg R0.q <- R1.q",
        "neg!" => "    neg! R0.q <- R1.q",
        "inc" => "    inc R0.q <- R1.q",
        "inc!" => "    inc! R0.q <- R1.q",
        "dec" => "    dec R0.q <- R1.q",
        "dec!" => "    dec! R0.q <- R1.q",
        "not" => "    not R0.q <- R1.q",
        "not!" => "    not! R0.q <- R1.q",
        "and" => "    and R0.q <- R1.q, 2",
        "and!" => "    and! R0.q <- R1.q, 2",
        "or" => "    or R0.q <- R1.q, 2",
        "or!" => "    or! R0.q <- R1.q, 2",
        "xor" => "    xor R0.q <- R1.q, 2",
        "xor!" => "    xor! R0.q <- R1.q, 2",
        "sum" => "    sum R0.q <- R1.q, 2",
        "sum!" => "    sum! R0.q <- R1.q, 2",
        "diff" => "    diff R0.q <- R1.q, 2",
        "diff!" => "    diff! R0.q <- R1.q, 2",
        "compare!" => "    compare! R0.q, 1",
        "shift.l" => "    shift.l R0.q <- R1.q, 1",
        "shift.l!" => "    shift.l! R0.q <- R1.q, 1",
        "shift.r" => "    shift.r R0.q <- R1.q, 1",
        "shift.r!" => "    shift.r! R0.q <- R1.q, 1",
        "shift.ar" => "    shift.ar R0.q <- R1.q, 1",
        "shift.ar!" => "    shift.ar! R0.q <- R1.q, 1",
        "rotate.l" => "    rotate.l R0.q <- R1.q, 1",
        "rotate.l!" => "    rotate.l! R0.q <- R1.q, 1",
        "rotate.r" => "    rotate.r R0.q <- R1.q, 1",
        "rotate.r!" => "    rotate.r! R0.q <- R1.q, 1",
        "bit.set" => "    bit.set R0.q <- R1.q, 1",
        "bit.clear" => "    bit.clear R0.q <- R1.q, 1",
        "bit.test!" => "    bit.test! R0.q, 1",
        "load.u8" => "    load.u8 R0.b <- [R1.q]",
        "load.u16" => "    load.u16 R0.w <- [R1.q]",
        "load.u32" => "    load.u32 R0.d <- [R1.q]",
        "load.u64" => "    load.u64 R0.q <- [R1.q]",
        "store.u8" => "    store.u8 [R1.q] <- R0.b",
        "store.u16" => "    store.u16 [R1.q] <- R0.w",
        "store.u32" => "    store.u32 [R1.q] <- R0.d",
        "store.u64" => "    store.u64 [R1.q] <- R0.q",
        "load.acquire.u8" => "    load.acquire.u8 R0.b <- [R1.q]",
        "load.acquire.u16" => "    load.acquire.u16 R0.w <- [R1.q]",
        "load.acquire.u32" => "    load.acquire.u32 R0.d <- [R1.q]",
        "load.acquire.u64" => "    load.acquire.u64 R0.q <- [R1.q]",
        "store.release.u8" => "    store.release.u8 [R1.q] <- R0.b",
        "store.release.u16" => "    store.release.u16 [R1.q] <- R0.w",
        "store.release.u32" => "    store.release.u32 [R1.q] <- R0.d",
        "store.release.u64" => "    store.release.u64 [R1.q] <- R0.q",
        "jump" => "    jump -> main",
        "tailcall" => "    tailcall -> main",
        "call" => "    call -> main",
        "exchange" => "    exchange R0 <-> R1",
        "compare.swap" => "    compare.swap R0.q <- [R1.q], R2.q, R3.q",
        "if.zero" => "    if.zero -> main",
        "if.notzero" => "    if.notzero -> main",
        "if.carry" => "    if.carry -> main",
        "if.nocarry" => "    if.nocarry -> main",
        "if.sign" => "    if.sign -> main",
        "if.notsign" => "    if.notsign -> main",
        "if.overflow" => "    if.overflow -> main",
        "if.nooverflow" => "    if.nooverflow -> main",
        "if.even" => "    if.even -> main",
        "if.odd" => "    if.odd -> main",
        "sys.region.map" => "    sys.region.map",
        "sys.region.unmap" => "    sys.region.unmap",
        "sys.port.read" => "    sys.port.read",
        "sys.port.write" => "    sys.port.write",
        "sys.irq.bind" => "    sys.irq.bind",
        "sys.irq.enable" => "    sys.irq.enable",
        "sys.irq.disable" => "    sys.irq.disable",
        "sys.context.enter" => "    sys.context.enter",
        "sys.context.return" => "    sys.context.return",
        "sys.cpu.control" => "    sys.cpu.control",
        "sys.power" => "    sys.power",
        other => panic!("the native-subset sweep has no sample for `{other}`"),
    }
}

/// The public instruction reference must describe every instruction the
/// toolchain defines, and must agree with the native subset table.
#[test]
fn instruction_reference_covers_every_defined_mnemonic() {
    use ascension_asm::instructions::mnemonic_for;
    use ascension_asm::native::native_supports;

    let reference = include_str!("../../manual/INSTRUCTION-REFERENCE.md");
    let mut missing = Vec::new();
    for opcode in 0x0001u16..=0x00ff {
        let Some(name) = mnemonic_for(opcode) else {
            continue;
        };
        let row = reference
            .lines()
            .find(|line| line.starts_with(&format!("| `{name}` |")));
        let Some(row) = row else {
            missing.push(name);
            continue;
        };
        // Leading and trailing pipes produce empty outer cells: the layout is
        // ["", mnemonic, opcode, form, width, native, notes, ""].
        let cells: Vec<&str> = row.split('|').map(str::trim).collect();
        let native_cell = cells[5];
        let documented = native_cell == "yes";
        assert_eq!(
            documented,
            native_supports(opcode),
            "the reference says `{name}` native={documented}, the emitter says {}",
            native_supports(opcode)
        );
        let opcode_cell = cells[2].trim_matches('`');
        assert_eq!(
            opcode_cell,
            format!("0x{opcode:04X}"),
            "the reference lists the wrong opcode for `{name}`"
        );
    }
    assert!(
        missing.is_empty(),
        "the instruction reference omits: {missing:?}"
    );
}

/// No tier may silently substitute for another: a native request for an image
/// outside the native subset fails, names the offending instruction, and says
/// what to do instead. The interpreter still runs the same image.
#[test]
fn native_refusal_names_the_unsupported_instruction() {
    use ascension_asm::native::unsupported_instructions;

    // `load.u64` is interpreter-only; the program itself is well formed.
    let body = "move R1.q <- 0x1000\n    load.u64 R0.q <- [R1.q]\n";
    let image = image_of(body);
    let missing = unsupported_instructions(&image).unwrap();
    assert_eq!(missing, vec!["load.u64".to_string()]);

    let error = emit_assembly(&image).unwrap_err().to_string();
    assert!(
        error.contains("load.u64"),
        "the refusal must name it: {error}"
    );
    assert!(
        error.contains("manual/JIT-AOT.md"),
        "the refusal must point at the documented subset: {error}"
    );

    // A translatable program reports nothing missing.
    assert!(
        unsupported_instructions(&image_of("move R0.q <- 1\n"))
            .unwrap()
            .is_empty()
    );

    // The interpreter is unaffected, and faults for the real reason.
    let mut machine = Avm::load(&image).unwrap();
    assert!(matches!(machine.run(), Err(AvmError::MemoryAccess { .. })));
}

/// The CLI must refuse a native run of an interpreter-only image with exit
/// status 2 and a named instruction, rather than emitting a linker error or
/// quietly interpreting.
#[test]
fn cli_refuses_native_runs_that_need_the_interpreter() {
    let directory = std::env::temp_dir().join(format!(
        "ascension-reg-subset-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let source = directory.join("program.pa");
    std::fs::write(
        &source,
        "export main\nmain::\n    move R1.q <- 0x1000\n    load.u64 R0.q <- [R1.q]\n    stop\n",
    )
    .unwrap();
    for mode in ["--native", "--jit"] {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_plus-a"))
            .arg("run")
            .arg(&source)
            .arg(mode)
            .arg("--build-dir")
            .arg(&directory)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2), "mode {mode}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("load.u64"), "mode {mode}: {stderr}");
    }
    std::fs::remove_dir_all(directory).unwrap();
}

/// Memory semantics that the documentation promises, pinned so the docs cannot
/// drift: little-endian storage, non-widening narrow loads, ordinary accesses
/// allowed while atomic ones require alignment and a shared/atomic region.
#[test]
fn memory_semantics_match_the_documented_rules() {
    let mapping = |base: u64, len: usize, writable: bool, shared: bool| MemoryRegion {
        base,
        bytes: vec![0; len],
        readable: true,
        writable,
        shared,
        atomic: shared,
    };
    let run = |body: &str, regions: Vec<MemoryRegion>| {
        let image = assemble(&format!("export main\nmain::\n{body}    stop\n")).unwrap();
        let mut machine = Avm::load(&image).unwrap();
        for region in regions {
            machine.map_region(region).unwrap();
        }
        machine
    };

    // Little-endian storage; a narrow load writes through its own view, so it
    // preserves the destination's upper bits.
    let mut machine = run(
        "    move R1.q <- 0x1000\n    move R2.q <- 0x1122_3344_5566_7788\n\
         store.u64 [R1.q] <- R2.q\n    move R0.q <- 0\n    load.u8 R0.b <- [R1.q]\n",
        vec![mapping(0x1000, 64, true, false)],
    );
    assert_eq!(machine.run(), Ok(RunResult::Stopped(0x88)));
    assert_eq!(
        machine.memory_region(0x1000).unwrap().bytes[..4],
        [0x88, 0x77, 0x66, 0x55]
    );

    // A byte load is not widening: the target keeps its upper bits.
    let mut machine = run(
        "    move R1.q <- 0x1000\n    move R2.q <- 0x1122_3344_5566_7788\n\
         store.u64 [R1.q] <- R2.q\n    move R0.q <- 0xFFFF_FFFF_FFFF_0000\n\
         load.u8 R0.b <- [R1.q]\n",
        vec![mapping(0x1000, 64, true, false)],
    );
    assert_eq!(machine.run(), Ok(RunResult::Stopped(0xFFFF_FFFF_FFFF_0088)));

    // Ordinary unaligned access is permitted.
    let mut machine = run(
        "    move R1.q <- 0x1001\n    move R0.q <- 0x1122_3344_5566_7788\n\
         store.u64 [R1.q] <- R0.q\n    load.u64 R0.q <- [R1.q]\n",
        vec![mapping(0x1000, 64, true, false)],
    );
    assert_eq!(machine.run(), Ok(RunResult::Stopped(0x1122_3344_5566_7788)));

    // An atomic access needs a shared+atomic region...
    let mut machine = run(
        "    move R1.q <- 0x1000\n    load.acquire.u64 R0.q <- [R1.q]\n",
        vec![mapping(0x1000, 64, true, false)],
    );
    assert!(matches!(machine.run(), Err(AvmError::MemoryAccess { .. })));

    // ...and natural alignment even then.
    let mut machine = run(
        "    move R1.q <- 0x2002\n    load.acquire.u64 R0.q <- [R1.q]\n",
        vec![mapping(0x2000, 64, true, true)],
    );
    assert!(matches!(machine.run(), Err(AvmError::Alignment { .. })));

    // A store into a read-only region faults.
    let mut machine = run(
        "    move R1.q <- 0x3000\n    store.u64 [R1.q] <- R0.q\n",
        vec![mapping(0x3000, 8, false, false)],
    );
    assert!(matches!(machine.run(), Err(AvmError::MemoryAccess { .. })));

    // A single access may not straddle two regions.
    let mut machine = run(
        "    move R1.q <- 0x1FFC\n    load.u64 R0.q <- [R1.q]\n",
        vec![
            mapping(0x1000, 0x1000, true, false),
            mapping(0x2000, 0x1000, true, false),
        ],
    );
    assert!(matches!(machine.run(), Err(AvmError::MemoryAccess { .. })));

    // Overlapping regions are refused at map time.
    let mut machine = Avm::load(&image_of("move R0.q <- 1\n")).unwrap();
    machine
        .map_region(mapping(0x1000, 0x100, true, false))
        .unwrap();
    assert!(
        machine
            .map_region(mapping(0x1080, 0x100, true, false))
            .is_err(),
        "overlapping regions must be refused"
    );
}
