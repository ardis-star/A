use ascension_asm::assemble;
use ascension_asm::avm::{Avm, AvmError, MemoryRegion, RunResult};
use ascension_asm::bytecode::crc32c;

fn load(source: &str) -> Avm {
    let image = assemble(source).unwrap();
    Avm::load(&image).unwrap()
}

fn run(source: &str) -> (Avm, Result<RunResult, AvmError>) {
    let image = assemble(source).unwrap();
    let mut machine = Avm::load(&image).unwrap();
    let result = machine.run();
    (machine, result)
}

fn stopped(source: &str) -> u64 {
    let (_, result) = run(source);
    match result {
        Ok(RunResult::Stopped(status)) => status,
        Ok(RunResult::Returned) => panic!("expected stopped, got returned"),
        Err(error) => panic!("expected stopped, got {error:?}"),
    }
}

fn returned(source: &str) {
    let (_, result) = run(source);
    assert!(matches!(result, Ok(RunResult::Returned)), "{result:?}");
}

fn faults(source: &str, error: AvmError) {
    let (_, result) = run(source);
    assert_eq!(result, Err(error), "{result:?}");
}

fn region(base: u64, len: usize) -> MemoryRegion {
    MemoryRegion {
        base,
        bytes: vec![0; len],
        readable: true,
        writable: true,
        shared: false,
        atomic: false,
    }
}

#[test]
fn avm_nop_break_return_stop() {
    returned("export main\nmain::\n    nop\n    nop\n    return\n");
    faults(
        "export main\nmain::\n    break\n    stop\n",
        AvmError::Breakpoint,
    );
    returned("export main\nmain::\n    return\n");
    assert_eq!(
        stopped("export main\nmain::\n    move R0.q <- 42\n    stop\n"),
        42
    );
    assert_eq!(stopped("export main\nmain::\n    stop\n"), 0);
}

#[test]
fn checked_in_examples_execute_in_avm() {
    let arithmetic = include_str!("../../examples/arithmetic.pa");
    let registers = include_str!("../../examples/registers.pa");
    let hello_world = include_str!("../../examples/hello_world.pa");
    let control_flow = include_str!("../../examples/control_flow.pa");
    assert_eq!(stopped(arithmetic), 125);
    assert_eq!(stopped(registers), 100);
    assert_eq!(stopped(hello_world), 0);
    assert_eq!(stopped(control_flow), 42);
}

#[test]
fn avm_move_widths_and_merge() {
    assert_eq!(
        stopped(
            "export main\nmain::\n    move R0.b <- 0xAB\n    move R1.w <- 0xCDEF\n    move R2.d <- 0xFFFF_FFFF\n    move R3.q <- 0x1122_3344_5566_7788\n    move R0.q <- R3.q\n    stop\n",
        ),
        0x1122_3344_5566_7788
    );
    let mut m = load(
        "export main\nmain::\n    move R0.q <- 0xFFFF_FFFF_FFFF_FFFF\n    move R0.b <- 0x12\n    move R1.d <- 0xFFFF_FFFF\n    stop\n",
    );
    m.run().unwrap();
    assert_eq!(m.registers[0], 0xffff_ffff_ffff_ff12);
    assert_eq!(m.registers[1], 0x0000_0000_ffff_ffff);
}

#[test]
fn avm_extend_sign_and_zero() {
    let mut m = load(
        "export main\nmain::\n    move R0.b <- 0x80\n    extend.s R1.q <- R0.b\n    extend.z R2.q <- R0.b\n    stop\n",
    );
    m.run().unwrap();
    assert_eq!(m.registers[1], u64::MAX - 127);
    assert_eq!(m.registers[2], 0x80);
    let mut m =
        load("export main\nmain::\n    move R0.w <- 0x8000\n    extend.s R1.d <- R0.w\n    stop\n");
    m.run().unwrap();
    assert_eq!(m.registers[1], 0xFFFF_8000);
    let mut m =
        load("export main\nmain::\n    move R0.b <- 0x7F\n    extend.s R1.q <- R0.b\n    stop\n");
    m.run().unwrap();
    assert_eq!(m.registers[1], 0x7F);
}

#[test]
fn avm_select_predicates() {
    let mut m = load(
        "export main\nmain::\n    move R0.q <- 0\n    compare! R0.q, 0\n    select R1.q <- zero, 10, 20\n    move R0.q <- R1.q\n    stop\n",
    );
    m.run().unwrap();
    assert_eq!(m.registers[0], 10);
    let mut m = load(
        "export main\nmain::\n    move R0.q <- 5\n    compare! R0.q, 0\n    select R1.q <- notzero, 10, 20\n    move R0.q <- R1.q\n    stop\n",
    );
    m.run().unwrap();
    assert_eq!(m.registers[0], 10);
    let mut m = load(
        "export main\nmain::\n    move R0.q <- 0\n    sum! R1.q <- R0.q, 0\n    select R2.q <- even, 7, 9\n    select R3.q <- odd, 7, 9\n    move R0.q <- R2.q\n    stop\n",
    );
    m.run().unwrap();
    assert_eq!(m.registers[0], 7);
    let mut m = load(
        "export main\nmain::\n    move R0.q <- 1\n    sum! R1.q <- R0.q, 0\n    select R2.q <- even, 7, 9\n    select R3.q <- odd, 7, 9\n    move R0.q <- R3.q\n    stop\n",
    );
    m.run().unwrap();
    assert_eq!(m.registers[0], 7);
}

#[test]
fn avm_select_preserves_flags() {
    let mut m = load(
        "export main\nmain::\n    move R0.q <- 0\n    sum! R1.q <- R0.q, 0\n    select R2.q <- zero, 3, 4\n    stop\n",
    );
    m.run().unwrap();
    assert_eq!(m.registers[2], 3);
    assert_eq!(m.flags & (1 << 1), 0);
}

#[test]
fn avm_sum_diff_flags() {
    assert_eq!(
        stopped(
            "export main\nmain::\n    move R0.q <- 1\n    sum! R1.q <- R0.q, 2\n    move R0.q <- R1.q\n    stop\n"
        ),
        3
    );
    assert_eq!(
        stopped(
            "export main\nmain::\n    move R0.q <- 5\n    diff! R1.q <- R0.q, 3\n    move R0.q <- R1.q\n    stop\n"
        ),
        2
    );
    let mut m =
        load("export main\nmain::\n    move R0.b <- 255\n    sum! R1.b <- R0.b, 1\n    stop\n");
    m.run().unwrap();
    assert_eq!(m.registers[1], 0);
    assert_ne!(m.flags & 1, 0);
    assert_ne!(m.flags & (1 << 1), 0);
    assert_ne!(m.flags & (1 << 4), 0);
    let mut m = load(
        "export main\nmain::\n    move R0.q <- 0x4000_0000_0000_0000\n    sum! R1.q <- R0.q, R0.q\n    stop\n",
    );
    m.run().unwrap();
    assert_ne!(m.flags & (1 << 3), 0);
    let mut m =
        load("export main\nmain::\n    move R0.b <- 0\n    diff! R1.b <- R0.b, 1\n    stop\n");
    m.run().unwrap();
    assert_ne!(m.flags & (1 << 1), 0);
    assert_ne!(m.flags & (1 << 2), 0);
}

#[test]
fn avm_compare_flag_only() {
    let mut m = load(
        "export main\nmain::\n    move R0.q <- 5\n    compare! R0.q, 5\n    move R1.q <- 0\n    if.zero -> done\n    move R1.q <- 1\n done::\n    stop\n",
    );
    m.run().unwrap();
    assert_eq!(m.registers[1], 0);
    assert_ne!(m.flags & 1, 0);
    let mut m = load(
        "export main\nmain::\n    move R0.q <- 5\n    compare! R0.q, 3\n    move R1.q <- 0\n    if.notzero -> done\n    move R1.q <- 1\n done::\n    stop\n",
    );
    m.run().unwrap();
    assert_eq!(m.registers[1], 0);
    assert_eq!(m.flags & 1, 0);
}

#[test]
fn avm_sumc_diffc_carry() {
    let mut m = load(
        "export main\nmain::\n    move R0.b <- 255\n    sum! R2.b <- R0.b, 1\n    sumc! R3.b <- R0.b, 0\n    stop\n",
    );
    m.flags = 1;
    m.run().unwrap();
    assert_eq!(m.registers[2], 0);
    assert_ne!(m.flags & (1 << 1), 0);
    assert_eq!(m.registers[3], 0);
    assert_ne!(m.flags & (1 << 1), 0);
    let mut m =
        load("export main\nmain::\n    move R0.b <- 0\n    diffc! R1.b <- R0.b, 0\n    stop\n");
    m.flags = 1 << 1;
    m.run().unwrap();
    assert_eq!(m.registers[1], 255);
    assert_ne!(m.flags & (1 << 1), 0);
    assert_ne!(m.flags & (1 << 2), 0);
}

#[test]
fn avm_mul_truncating_flags() {
    assert_eq!(
        stopped(
            "export main\nmain::\n    move R0.b <- 20\n    mul! R1.b <- R0.b, 13\n    move R0.q <- R1.q\n    stop\n"
        ),
        4
    );
    assert_eq!(
        stopped(
            "export main\nmain::\n    move R0.q <- 0xFFFF_FFFF_FFFF\n    mul! R1.q <- R0.q, 4\n    move R0.q <- R1.q\n    stop\n"
        ),
        0x3_FFFF_FFFF_FFFC
    );
    let mut m = load(
        "export main\nmain::\n    move R0.q <- 5\n    sum! R1.q <- R0.q, 5\n    mul R2.q <- R0.q, R1.q\n    stop\n",
    );
    m.run().unwrap();
    assert_eq!(m.registers[2], 50);
    assert_eq!(m.flags & 1, 0);
}

#[test]
fn avm_checked_sum_fault_no_commit() {
    faults(
        "export main\nmain::\n    move R0.b <- 127\n    checked.sum R1.b <- R0.b, 1\n    stop\n",
        AvmError::IntegerOverflow,
    );
    let mut m = load(
        "export main\nmain::\n    move R0.b <- 127\n    checked.sum R1.b <- R0.b, 1\n    stop\n",
    );
    assert_eq!(m.run().unwrap_err(), AvmError::IntegerOverflow);
    assert_eq!(m.registers[1], 0);
    assert_eq!(m.pc, 1);
    assert_eq!(
        stopped(
            "export main\nmain::\n    move R0.b <- 100\n    checked.sum R1.b <- R0.b, 20\n    move R0.q <- R1.q\n    stop\n",
        ),
        120
    );
}

#[test]
fn avm_neg_inc_dec_flags() {
    assert_eq!(
        stopped(
            "export main\nmain::\n    move R0.b <- 5\n    neg! R1.b <- R0.b\n    move R0.q <- R1.q\n    stop\n"
        ),
        0xFB
    );
    assert_eq!(
        stopped(
            "export main\nmain::\n    move R0.b <- 254\n    inc! R1.b <- R0.b\n    move R0.q <- R1.q\n    stop\n"
        ),
        255
    );
    assert_eq!(
        stopped(
            "export main\nmain::\n    move R0.b <- 255\n    inc! R1.b <- R0.b\n    move R0.q <- R1.q\n    stop\n"
        ),
        0
    );
    assert_eq!(
        stopped(
            "export main\nmain::\n    move R0.b <- 1\n    dec! R1.b <- R0.b\n    move R0.q <- R1.q\n    stop\n"
        ),
        0
    );
    assert_eq!(
        stopped(
            "export main\nmain::\n    move R0.b <- 0\n    dec! R1.b <- R0.b\n    move R0.q <- R1.q\n    stop\n"
        ),
        255
    );
    let mut m =
        load("export main\nmain::\n    move R0.b <- 0x80\n    neg! R1.b <- R0.b\n    stop\n");
    m.run().unwrap();
    assert_ne!(m.flags & (1 << 3), 0);
    let mut m =
        load("export main\nmain::\n    move R0.b <- 0x7F\n    inc! R1.b <- R0.b\n    stop\n");
    m.run().unwrap();
    assert_ne!(m.flags & (1 << 3), 0);
}

#[test]
fn avm_logical_flags() {
    assert_eq!(
        stopped(
            "export main\nmain::\n    move R0.q <- 0xFF\n    and! R1.q <- R0.q, 0x0F\n    move R0.q <- R1.q\n    stop\n"
        ),
        0x0F
    );
    assert_eq!(
        stopped(
            "export main\nmain::\n    move R0.q <- 0x0F\n    or! R1.q <- R0.q, 0xF0\n    move R0.q <- R1.q\n    stop\n"
        ),
        0xFF
    );
    assert_eq!(
        stopped(
            "export main\nmain::\n    move R0.q <- 0xFF\n    xor! R1.q <- R0.q, 0xFF\n    move R0.q <- R1.q\n    stop\n"
        ),
        0
    );
    let mut m = load(
        "export main\nmain::\n    move R0.q <- 5\n    sum! R1.q <- R0.q, 5\n    and R2.q <- R0.q, R1.q\n    stop\n",
    );
    m.run().unwrap();
    assert_eq!(m.registers[2], 5 & 10);
    assert_eq!(m.flags & 1, 0);
}

#[test]
fn avm_shifts() {
    assert_eq!(
        stopped(
            "export main\nmain::\n    move R0.b <- 1\n    shift.l! R1.b <- R0.b, 8\n    move R0.q <- R1.q\n    stop\n"
        ),
        1
    );
    assert_eq!(
        stopped(
            "export main\nmain::\n    move R0.b <- 0x80\n    shift.l! R1.b <- R0.b, 1\n    move R0.q <- R1.q\n    stop\n"
        ),
        0
    );
    let mut m = load(
        "export main\nmain::\n    move R0.b <- 0x80\n    shift.l! R1.b <- R0.b, 1\n    stop\n",
    );
    m.run().unwrap();
    assert_ne!(m.flags & (1 << 1), 0);
    assert_eq!(
        stopped(
            "export main\nmain::\n    move R0.b <- 0x01\n    shift.r! R1.b <- R0.b, 1\n    move R0.q <- R1.q\n    stop\n"
        ),
        0
    );
    assert_eq!(
        stopped(
            "export main\nmain::\n    move R0.b <- 0x01\n    shift.r! R1.b <- R0.b, 9\n    move R0.q <- R1.q\n    stop\n"
        ),
        0
    );
    assert_eq!(
        stopped(
            "export main\nmain::\n    move R0.b <- 0x80\n    shift.ar! R1.b <- R0.b, 1\n    move R0.q <- R1.q\n    stop\n"
        ),
        0xC0
    );
    assert_eq!(
        stopped(
            "export main\nmain::\n    move R0.q <- 0xFFFF_FFFF_FFFF_FFFF\n    shift.ar! R1.q <- R0.q, 1\n    move R0.q <- R1.q\n    stop\n"
        ),
        u64::MAX
    );
}

#[test]
fn avm_rotates() {
    assert_eq!(
        stopped(
            "export main\nmain::\n    move R0.b <- 0x80\n    rotate.l! R1.b <- R0.b, 1\n    move R0.q <- R1.q\n    stop\n"
        ),
        0x01
    );
    assert_eq!(
        stopped(
            "export main\nmain::\n    move R0.b <- 0x01\n    rotate.r! R1.b <- R0.b, 1\n    move R0.q <- R1.q\n    stop\n"
        ),
        0x80
    );
    assert_eq!(
        stopped(
            "export main\nmain::\n    move R0.b <- 0x80\n    rotate.l! R1.b <- R0.b, 9\n    move R0.q <- R1.q\n    stop\n"
        ),
        0x01
    );
    let mut m = load(
        "export main\nmain::\n    move R0.b <- 0x80\n    rotate.l! R1.b <- R0.b, 1\n    stop\n",
    );
    m.run().unwrap();
    assert_ne!(m.flags & (1 << 1), 0);
}

#[test]
fn avm_bit_ops() {
    assert_eq!(
        stopped(
            "export main\nmain::\n    move R0.b <- 0\n    bit.set R1.b <- R0.b, 5\n    move R0.q <- R1.q\n    stop\n"
        ),
        0x20
    );
    assert_eq!(
        stopped(
            "export main\nmain::\n    move R0.b <- 0xFF\n    bit.clear R1.b <- R0.b, 5\n    move R0.q <- R1.q\n    stop\n"
        ),
        0xDF
    );
    let mut m =
        load("export main\nmain::\n    move R0.b <- 0x20\n    bit.test! R0.b, 5\n    stop\n");
    m.run().unwrap();
    assert_eq!(m.flags & 1, 0);
    let mut m =
        load("export main\nmain::\n    move R0.b <- 0x00\n    bit.test! R0.b, 5\n    stop\n");
    m.run().unwrap();
    assert_ne!(m.flags & 1, 0);
}

#[test]
fn avm_memory_load_store() {
    let mut m = load(
        "export main\nmain::\n    move R1.q <- 0x1000\n    move R0.q <- 0x1122_3344_5566_7788\n    store.u64 [R1.q] <- R0.q\n    load.u64 R2.q <- [R1.q]\n    move R0.q <- R2.q\n    stop\n",
    );
    m.map_region(region(0x1000, 16)).unwrap();
    m.run().unwrap();
    assert_eq!(m.registers[0], 0x1122_3344_5566_7788);
    assert_eq!(
        &m.memory_region(0x1000).unwrap().bytes[..8],
        &[0x88, 0x77, 0x66, 0x55, 0x44, 0x33, 0x22, 0x11]
    );
    let mut m = load(
        "export main\nmain::\n    move R1.q <- 0x2000\n    move R0.w <- 0x1234\n    store.u16 [R1.q] <- R0.w\n    load.u8 R2.b <- [R1.q + 1]\n    move R0.q <- R2.q\n    stop\n",
    );
    m.map_region(region(0x2000, 16)).unwrap();
    m.run().unwrap();
    assert_eq!(m.registers[0], 0x12);
}

#[test]
fn avm_memory_permissions() {
    let mut read_only = load(
        "export main\nmain::\n    move R1.q <- 0x1000\n    store.u16 [R1.q] <- R0.w\n    stop\n",
    );
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
    let mut unmapped = load(
        "export main\nmain::\n    move R1.q <- 0x1000\n    load.u16 R2.w <- [R1.q]\n    stop\n",
    );
    assert!(matches!(
        unmapped.run(),
        Err(AvmError::MemoryAccess {
            address: 0x1000,
            length: 2
        })
    ));
}

#[test]
fn avm_atomic_requirements() {
    let mut m = load(
        "export main\nmain::\n    move R1.q <- 0x1000\n    move R0.d <- 0x1234_5678\n    store.release.u32 [R1.q] <- R0.d\n    load.acquire.u32 R2.d <- [R1.q]\n    move R0.q <- R2.q\n    stop\n",
    );
    m.map_region(MemoryRegion {
        base: 0x1000,
        bytes: vec![0; 8],
        readable: true,
        writable: true,
        shared: true,
        atomic: true,
    })
    .unwrap();
    m.run().unwrap();
    assert_eq!(m.registers[0], 0x1234_5678);
    let mut misaligned = load(
        "export main\nmain::\n    move R1.q <- 0x1001\n    load.acquire.u32 R2.d <- [R1.q]\n    stop\n",
    );
    misaligned
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
        misaligned.run(),
        Err(AvmError::Alignment {
            address: 0x1001,
            alignment: 4
        })
    ));
    let mut unshared = load(
        "export main\nmain::\n    move R1.q <- 0x1000\n    store.release.u32 [R1.q] <- R0.d\n    stop\n",
    );
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
fn avm_exchange() {
    let mut m = load(
        "export main\nmain::\n    move R0.q <- 1\n    move R1.q <- 2\n    exchange R0 <-> R1\n    move R0.q <- R0.q\n    stop\n",
    );
    m.run().unwrap();
    assert_eq!(m.registers[0], 2);
    assert_eq!(m.registers[1], 1);
    let mut m = load(
        "export main\nmain::\n    move R0.q <- 0xFFFF_FFFF_FFFF_FFFF\n    move R1.q <- 0\n    exchange R0 <-> R1\n    stop\n",
    );
    m.run().unwrap();
    assert_eq!(m.registers[0], 0);
    assert_eq!(m.registers[1], 0xFFFF_FFFF_FFFF_FFFF);
}

#[test]
fn avm_branches() {
    assert_eq!(
        stopped(
            "export main\nmain::\n    move R0.q <- 1\n    jump -> done\n    move R0.q <- 99\n done::\n    stop\n",
        ),
        1
    );
    assert_eq!(
        stopped(
            "export main\nmain::\n    move R0.q <- 0\n    compare! R0.q, 0\n    if.zero -> taken\n    move R0.q <- 99\n    jump -> done\n taken::\n    move R0.q <- 42\n done::\n    stop\n",
        ),
        42
    );
    assert_eq!(
        stopped(
            "export main\nmain::\n    move R0.q <- 5\n    compare! R0.q, 0\n    if.notzero -> taken\n    move R0.q <- 99\n    jump -> done\n taken::\n    move R0.q <- 42\n done::\n    stop\n",
        ),
        42
    );
    assert_eq!(
        stopped(
            "export main\nmain::\n    move R0.q <- 0x80\n    diff! R1.q <- R0.q, 0x81\n    if.sign -> taken\n    move R0.q <- 99\n    jump -> done\n taken::\n    move R0.q <- 42\n done::\n    stop\n",
        ),
        42
    );
    assert_eq!(
        stopped(
            "export main\nmain::\n    move R0.q <- 5\n    diff! R1.q <- R0.q, 3\n    if.notsign -> taken\n    move R0.q <- 99\n    jump -> done\n taken::\n    move R0.q <- 42\n done::\n    stop\n",
        ),
        42
    );
    assert_eq!(
        stopped(
            "export main\nmain::\n    move R0.q <- 0x7FFF_FFFF_FFFF_FFFF\n    sum! R1.q <- R0.q, 1\n    if.overflow -> taken\n    move R0.q <- 99\n    jump -> done\n taken::\n    move R0.q <- 42\n done::\n    stop\n",
        ),
        42
    );
    assert_eq!(
        stopped(
            "export main\nmain::\n    move R0.q <- 1\n    sum! R1.q <- R0.q, 1\n    if.nooverflow -> taken\n    move R0.q <- 99\n    jump -> done\n taken::\n    move R0.q <- 42\n done::\n    stop\n",
        ),
        42
    );
    assert_eq!(
        stopped(
            "export main\nmain::\n    move R0.q <- 1\n    sum! R1.q <- R0.q, 0xFFFF_FFFF_FFFF_FFFF\n    if.carry -> taken\n    move R0.q <- 99\n    jump -> done\n taken::\n    move R0.q <- 42\n done::\n    stop\n",
        ),
        42
    );
    assert_eq!(
        stopped(
            "export main\nmain::\n    move R0.q <- 5\n    diff! R1.q <- R0.q, 3\n    if.nocarry -> taken\n    move R0.q <- 99\n    jump -> done\n taken::\n    move R0.q <- 42\n done::\n    stop\n",
        ),
        42
    );
    assert_eq!(
        stopped(
            "export main\nmain::\n    move R0.q <- 3\n    sum! R1.q <- R0.q, 0\n    if.even -> taken\n    move R0.q <- 99\n    jump -> done\n taken::\n    move R0.q <- 42\n done::\n    stop\n",
        ),
        42
    );
    assert_eq!(
        stopped(
            "export main\nmain::\n    move R0.q <- 2\n    sum! R1.q <- R0.q, 0\n    if.odd -> taken\n    move R0.q <- 99\n    jump -> done\n taken::\n    move R0.q <- 42\n done::\n    stop\n",
        ),
        42
    );
    assert_eq!(
        stopped(
            "export main\nmain::\n    move R0.q <- 3\n    sum! R1.q <- R0.q, 0\n    if.odd -> taken\n    move R0.q <- 99\n    jump -> done\n taken::\n    move R0.q <- 42\n done::\n    stop\n",
        ),
        99
    );
}

#[test]
fn avm_call_return_tailcall() {
    assert_eq!(
        stopped(
            "export main\nmain::\n    move R0.q <- 41\n    call -> increment\n    stop\nincrement::\n    sum R0.q <- R0.q, 1\n    return\n",
        ),
        42
    );
    assert_eq!(
        stopped(
            "export main\nmain::\n    move R0.q <- 40\n    call -> wrapper\n    stop\nwrapper::\n    tailcall -> leaf\nleaf::\n    sum R0.q <- R0.q, 2\n    return\n",
        ),
        42
    );
    assert_eq!(
        stopped(
            "export main\nmain::\n    move R0.q <- 1\n    move R1.q <- 2\n    move R2.q <- 3\n    call -> sum3\n    stop\nsum3::\n    sum R0.q <- R0.q, R1.q\n    sum R0.q <- R0.q, R2.q\n    return\n",
        ),
        6
    );
    let mut m = load("export main\nmain::\n    call -> sub\n    stop\nsub::\n    return\n");
    m.run().unwrap();
    assert_eq!(m.pc, 2);
}

#[test]
fn avm_unassigned_rejected() {
    assert!(assemble("export main\nmain::\n    frobnicate R0.q <- R1.q\n    stop\n").is_err());
    assert!(assemble("export main\nmain::\n    move.block [R1] <- [R2], R3\n    stop\n").is_err());
    assert!(assemble("export main\nmain::\n    size R0.q <- main\n    stop\n").is_err());
}

#[test]
fn avm_unknown_opcode_rejected_by_verifier() {
    let mut image = assemble(include_str!("../tests/golden/minimal.a")).unwrap();
    image[260..262].copy_from_slice(&0xFFFF_u16.to_le_bytes());
    let code_crc = crc32c(&image[224..265]);
    image[40 + 2 * 32 + 28..40 + 2 * 32 + 32].copy_from_slice(&code_crc.to_le_bytes());
    assert!(
        ascension_asm::verifier::verify_image(&image)
            .unwrap_err()
            .to_string()
            .contains("unknown opcode")
    );
}

#[test]
fn avm_boundary_values() {
    assert_eq!(
        stopped(
            "export main\nmain::\n    move R0.b <- 255\n    sum! R1.b <- R0.b, 1\n    move R0.q <- R1.q\n    stop\n",
        ),
        0
    );
    assert_eq!(
        stopped(
            "export main\nmain::\n    move R0.w <- 65535\n    sum! R1.w <- R0.w, 1\n    move R0.q <- R1.q\n    stop\n",
        ),
        0
    );
    assert_eq!(
        stopped(
            "export main\nmain::\n    move R0.d <- 0xFFFF_FFFF\n    sum! R1.d <- R0.d, 1\n    move R0.q <- R1.q\n    stop\n",
        ),
        0
    );
    assert_eq!(
        stopped(
            "export main\nmain::\n    move R0.q <- 0xFFFF_FFFF_FFFF_FFFF\n    inc! R1.q <- R0.q\n    move R0.q <- R1.q\n    stop\n",
        ),
        0
    );
    assert_eq!(
        stopped(
            "export main\nmain::\n    move R0.b <- 0x80\n    sum! R1.b <- R0.b, 128\n    move R0.q <- R1.q\n    stop\n",
        ),
        0
    );
    assert_eq!(
        stopped(
            "export main\nmain::\n    move R0.b <- 0x80\n    extend.s R1.q <- R0.b\n    move R0.q <- R1.q\n    stop\n",
        ),
        u64::MAX - 127
    );
    assert_eq!(
        stopped(
            "export main\nmain::\n    move R0.q <- 0xFFFF_FFFF_FFFF_FFFF\n    diff! R1.q <- R0.q, 1\n    move R0.q <- R1.q\n    stop\n",
        ),
        0xFFFF_FFFF_FFFF_FFFE
    );
    assert_eq!(
        stopped(
            "export main\nmain::\n    move R0.q <- -0x8000_0000_0000_0000\n    neg! R1.q <- R0.q\n    move R0.q <- R1.q\n    stop\n",
        ),
        0x8000_0000_0000_0000
    );
}

#[test]
fn avm_no_partial_effect_on_fault() {
    let mut m = load(
        "export main\nmain::\n    move R0.q <- 0x7FFF_FFFF_FFFF_FFFF\n    checked.sum R1.q <- R0.q, 2\n    stop\n",
    );
    assert_eq!(m.run().unwrap_err(), AvmError::IntegerOverflow);
    assert_eq!(m.registers[1], 0);
    assert_eq!(m.pc, 1);
}

#[test]
fn avm_div_u() {
    // 100 / 7 = 14 remainder 2
    assert_eq!(
        stopped(
            "export main\nmain::\n    move R0.q <- 100\n    move R1.q <- 7\n    div.u R2.q, R3.q <- R0.q, R1.q\n    move R0.q <- R2.q\n    stop\n",
        ),
        14
    );
    let mut m = load(
        "export main\nmain::\n    move R0.q <- 100\n    move R1.q <- 7\n    div.u R2.q, R3.q <- R0.q, R1.q\n    stop\n",
    );
    m.run().unwrap();
    assert_eq!(m.registers[2], 14);
    assert_eq!(m.registers[3], 2);
    // Division by zero faults
    faults(
        "export main\nmain::\n    move R0.q <- 10\n    move R1.q <- 0\n    div.u R2.q, R3.q <- R0.q, R1.q\n    stop\n",
        AvmError::DivByZero,
    );
    // 8-bit width
    assert_eq!(
        stopped(
            "export main\nmain::\n    move R0.b <- 200\n    move R1.b <- 30\n    div.u R2.b, R3.b <- R0.b, R1.b\n    move R0.q <- R2.q\n    stop\n",
        ),
        6
    );
}

#[test]
fn avm_div_s() {
    // -100 / 7 = -14 remainder -2
    assert_eq!(
        stopped(
            "export main\nmain::\n    move R0.q <- -100\n    move R1.q <- 7\n    div.s R2.q, R3.q <- R0.q, R1.q\n    move R0.q <- R2.q\n    stop\n",
        ),
        0xFFFF_FFFF_FFFF_FFF2u64
    );
    // signed min / -1 faults
    faults(
        "export main\nmain::\n    move R0.q <- -0x8000_0000_0000_0000\n    move R1.q <- -1\n    div.s R2.q, R3.q <- R0.q, R1.q\n    stop\n",
        AvmError::DivByZero,
    );
    // division by zero faults
    faults(
        "export main\nmain::\n    move R0.q <- 10\n    move R1.q <- 0\n    div.s R2.q, R3.q <- R0.q, R1.q\n    stop\n",
        AvmError::DivByZero,
    );
}

#[test]
fn avm_catch_uncatch_raise() {
    // raise with handler transfers to handler
    assert_eq!(
        stopped(
            "export main\nmain::\n    move R0.q <- 42\n    catch -> handler\n    raise 42\n    stop\nhandler::\n    move R0.q <- R0.q\n    stop\n",
        ),
        42
    );
    // uncatch removes handler, fallthrough continues
    assert_eq!(
        stopped(
            "export main\nmain::\n    move R0.q <- 7\n    catch -> handler\n    uncatch\n    move R0.q <- 99\n    stop\nhandler::\n    move R0.q <- R0.q\n    stop\n",
        ),
        99
    );
    // raise with no active handler faults at runtime
    let image =
        assemble("export main\nmain::\n    move R0.q <- 3\n    raise 3\n    stop\n").unwrap();
    let mut machine = ascension_asm::avm::Avm::load_unverified(&image).unwrap();
    assert_eq!(machine.run().unwrap_err(), AvmError::InvalidHandler);
    // verifier rejects uncatch without catch
    let image = assemble("export main\nmain::\n    uncatch\n    stop\n").unwrap();
    assert!(
        ascension_asm::verifier::verify_image(&image)
            .unwrap_err()
            .to_string()
            .contains("underflow")
    );
}

#[test]
fn avm_flag_direction() {
    let mut m = load("export main\nmain::\n    flag.direction <- forward\n    stop\n");
    m.run().unwrap();
    assert_eq!(m.flags & (1 << 5), 0);
    let mut m = load("export main\nmain::\n    flag.direction <- backward\n    stop\n");
    m.run().unwrap();
    assert_ne!(m.flags & (1 << 5), 0);
    // flag.direction preserves other flags
    let mut m = load(
        "export main\nmain::\n    move R0.q <- 0xFFFF_FFFF_FFFF_FFFF\n    sum! R1.q <- R0.q, 1\n    flag.direction <- forward\n    stop\n",
    );
    m.run().unwrap();
    assert_ne!(m.flags & (1 << 1), 0); // carry still set
    assert_eq!(m.flags & (1 << 5), 0);
}

#[test]
fn avm_fence() {
    // fence is a no-op on sequential execution
    assert_eq!(
        stopped(
            "export main\nmain::\n    move R0.q <- 42\n    fence full\n    move R1.q <- R0.q\n    stop\n",
        ),
        42
    );
    assert_eq!(
        stopped("export main\nmain::\n    fence acquire\n    fence release\n    stop\n",),
        0
    );
}

#[test]
fn avm_address() {
    // address resolves a symbol to its function address
    let image = assemble("export main\nmain::\n    address R0.q <- main\n    stop\n").unwrap();
    let mut machine = Avm::load(&image).unwrap();
    machine.run().unwrap();
    // main is function 0, at code offset 36 (after descriptor table)
    assert_eq!(machine.registers[0], 36);
}

#[test]
fn avm_compare_swap() {
    // matching: old == expected, store desired, return old
    let mut m = load(
        "export main\nmain::\n    move R1.q <- 0x1000\n    move R0.q <- 0x1122_3344_5566_7788\n    store.u64 [R1.q] <- R0.q\n    move R0.q <- 0x1122_3344_5566_7788\n    move R2.q <- 0xFFFF_FFFF_FFFF_FFFF\n    compare.swap R3.q <- [R1.q], R0.q, R2.q\n    stop\n",
    );
    m.map_region(MemoryRegion {
        base: 0x1000,
        bytes: vec![0; 16],
        readable: true,
        writable: true,
        shared: true,
        atomic: true,
    })
    .unwrap();
    m.run().unwrap();
    assert_eq!(m.registers[3], 0x1122_3344_5566_7788);
    assert_eq!(m.memory_region(0x1000).unwrap().bytes[0], 0xFF);
    assert_eq!(m.memory_region(0x1000).unwrap().bytes[7], 0xFF);
    // mismatching: old != expected, no store, return old
    let mut m = load(
        "export main\nmain::\n    move R1.q <- 0x1000\n    move R0.q <- 0xDEAD_BEEF_DEAD_BEEF\n    move R2.q <- 0xFFFF_FFFF_FFFF_FFFF\n    compare.swap R3.q <- [R1.q], R0.q, R2.q\n    stop\n",
    );
    m.map_region(MemoryRegion {
        base: 0x1000,
        bytes: vec![0xFF; 16],
        readable: true,
        writable: true,
        shared: true,
        atomic: true,
    })
    .unwrap();
    m.run().unwrap();
    assert_eq!(m.registers[3], 0xFFFF_FFFF_FFFF_FFFF);
    assert_eq!(m.memory_region(0x1000).unwrap().bytes[0], 0xFF);
    assert_eq!(m.memory_region(0x1000).unwrap().bytes[7], 0xFF);
    // alignment fault
    let mut m = load(
        "export main\nmain::\n    move R1.q <- 0x1001\n    move R0.q <- 0\n    move R2.q <- 0\n    compare.swap R3.q <- [R1.q], R0.q, R2.q\n    stop\n",
    );
    m.map_region(MemoryRegion {
        base: 0x1000,
        bytes: vec![0; 16],
        readable: true,
        writable: true,
        shared: true,
        atomic: true,
    })
    .unwrap();
    assert!(matches!(
        m.run(),
        Err(AvmError::Alignment {
            address: 0x1001,
            alignment: 8
        })
    ));
}

#[test]
fn avm_sys_privilege_violation() {
    // sys.* outside privileged mode is a verifier error (no capabilities granted)
    let image = assemble("export main\nmain::\n    sys.region.map\n    stop\n").unwrap();
    assert!(
        ascension_asm::verifier::verify_image(&image)
            .unwrap_err()
            .to_string()
            .contains("capability")
    );
}

#[test]
fn avm_sys_unsupported_service() {
    // sys.* in privileged mode with capability -> UnsupportedService (no platform)
    let image = assemble("export main\nmain::\n    sys.region.map\n    stop\n").unwrap();
    let mut machine = ascension_asm::avm::Avm::load_unverified(&image).unwrap();
    machine.privileged = true;
    machine.capabilities = 1 << 0; // CAP_MEMORY_MAP
    assert_eq!(machine.run().unwrap_err(), AvmError::UnsupportedService);
}

#[test]
fn avm_raise_preserves_detail() {
    // raise sets code in R0, detail in R1 is preserved by handler
    assert_eq!(
        stopped(
            "export main\nmain::\n    move R0.q <- 10\n    move R1.q <- 99\n    catch -> handler\n    raise 10\n    stop\nhandler::\n    move R0.q <- R1.q\n    stop\n",
        ),
        99
    );
}
