# Changelog

All notable changes to the ASCENSION +A toolchain will be documented in this file.

## [1.0.0] - 2026-11-21

### Added
- **Official `.pa` Extension**: Full recognition and validation of `.pa` (Plus A) source files across the entire toolchain.
- **Unified `+a` CLI**: Production-ready command-line interface supporting direct execution (`+a file.pa`), `run`, `build`, `check`, `verify`, and `inspect`.
- **Golden Test Suite**: Bytecode golden verification tests ensuring deterministic binary output.
- **Complete Documentation Suite**: Architectural guides, instruction references, bytecode specifications, runtime guides, and limitation bounds.
- **Installation Scripts**: Automated `install.sh` and Cargo installation targets for `+a`, `plus-a`, and `aa`.

### Changed
- Migrated all examples and internal integration tests to `.pa`.
- Formalized V1.0 release metadata across `Cargo.toml` and CLI headers.

### Verified
- 100% passing test suite (133 tests) covering assembler, bytecode verification, AVM execution, JIT, and native AOT compilation.
- Zero compiler or Clippy warnings with `-D warnings`.
