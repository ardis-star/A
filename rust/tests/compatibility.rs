//! Images generated with the unmodified V1.1.0 base commit.
use ascension_asm::avm::{Avm, RunResult};
use ascension_asm::verifier::verify_image;

fn from_hex(text: &str) -> Vec<u8> {
    let text = text.trim();
    text.as_bytes()
        .chunks_exact(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}

#[test]
fn v11_example_bytecode_remains_compatible() {
    let examples = [
        (
            include_str!("../../tests/compatibility/v1.1.0/arithmetic.hex"),
            125,
        ),
        (
            include_str!("../../tests/compatibility/v1.1.0/control_flow.hex"),
            42,
        ),
        (
            include_str!("../../tests/compatibility/v1.1.0/first_program.hex"),
            0,
        ),
        (
            include_str!("../../tests/compatibility/v1.1.0/hello_world.hex"),
            0,
        ),
        (
            include_str!("../../tests/compatibility/v1.1.0/registers.hex"),
            100,
        ),
    ];
    for (fixture, status) in examples {
        let image = from_hex(fixture);
        verify_image(&image).unwrap();
        assert_eq!(
            Avm::load(&image).unwrap().run().unwrap(),
            RunResult::Stopped(status)
        );
    }
}
