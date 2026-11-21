# +A Core V2.0.0 acceptance checklist

This checklist records the Core V2.0.0 acceptance checks and their evidence.
Completion of development does not imply publication or a release tag.
Run commands from the repository root on Linux x86-64 unless noted otherwise.

| Requirement | Reproducible acceptance test | Result in this environment |
|---|---|---|
| Pinned build, tests and diagnostics | With rustup and the repository's `rust/rust-toolchain.toml`, run `cd rust && rustc --version && cargo --version && cargo clean && cargo build --locked --offline && cargo build --locked --offline --release && cargo test --locked --offline && cargo fmt -- --check && cargo clippy --locked --offline --all-targets --all-features -- -D warnings`. Require rustc 1.97.0 and all commands to succeed. | **Passed**: Rust 1.97.0 (2d8144b78), Cargo 1.97.0, rustdoc, rustfmt and Clippy. Debug/release builds, full tests, formatting and Clippy passed in a clean source snapshot. |
| Clean source build and installation | In a fresh checkout with the patch applied, run the pinned commands above; run `INSTALL_DIR=<empty-directory> ./install.sh`, then run the installed `+a --version`, `aa check examples/hello_world.pa`, and `plus-a run examples/arithmetic.pa`. Require 2.0.0, a successful check, and `stopped: 125`. | **Passed** in a clean source snapshot with Rust 1.97.0, including installation and the listed examples. |
| Unit, integration, runtime, verifier, CLI, JIT/AOT and doc-tests | `cargo test --manifest-path rust/Cargo.toml --locked --offline` must pass; report ignored tests and doc-test counts separately. | 182 passed, 1 ignored; doc-tests passed with 0 examples, using Rust 1.97.0. The ignored Python-reference test needs a reference tree absent from the checkout. |
| Prior bytecode compatibility | `cargo test --manifest-path rust/Cargo.toml --locked --offline --test compatibility`. Each of five V1.1.0 base-commit example images must verify and return its recorded result. | Passed with Rust 1.97.0. This is representative compatibility evidence, not an exhaustive guarantee about every historic or malformed image. |
| Untrusted image validation and resource behavior | `timeout 60s cargo test --manifest-path rust/Cargo.toml --locked --offline --test verifier verifier_large_multifunction_image_scales`; `cargo test --manifest-path rust/Cargo.toml --locked --offline --test verifier verifier_rejects_forged_symbol_count_before_allocation`. Require success; the stress image has 65,536 functions and 65,536 blocks. Run under OS resource limits in production. | Passed with Rust 1.97.0. Format size ceilings and these tests are not a general CPU/memory quota. |
| JIT memory boundary | `cargo test --manifest-path rust/Cargo.toml --locked --offline safe_jit_refuses_raw_memory_access` and `cargo test --manifest-path rust/Cargo.toml --locked --offline test_cli_jit_rejects_direct_host_memory`. Require safe library/CLI refusal and documented unsafe opt-in. | Passed with Rust 1.97.0. JIT is not a CPU/stack resource sandbox; use external isolation for untrusted code. |
| Native exit and fault handling | `cargo test --manifest-path rust/Cargo.toml --locked --offline --test cli native_run_distinguishes_status_70_from_runtime_faults_and_signals`. Normal stop 70 and 326 must report 70, while divide-by-zero and invalid-address termination must return an error. | Passed with Rust 1.97.0. Standalone native executables retain an inherently ambiguous process status 70 for a fault or a normal stop of 70. |
| Freestanding actual execution | Build release and run `python3 tests/freestanding/boot_test.py`. It requires `as`, `ld`, `objcopy`, `cc`, Python 3 and `qemu-system-x86_64` with BIOS firmware. Optionally set `PLUS_A`, `QEMU_SYSTEM_X86_64`, and `QEMU_DATA_DIR`. QEMU must run without an OS or AVM and show the +A code halted with RAX=42 and physical address 0x300000 containing 0x1122334455667788. | Passed using Rust 1.97.0 and QEMU 11.0.2 with TCG. The test uses a separate test-only Multiboot bootstrap, a stack, identity paging and long mode. |

## QEMU test design and limits

`tests/freestanding/kernel.pa` is compiled by +A into a freestanding ELF at
0x200000. The test also builds a relocatable object, compares its text against the
freestanding ELF, repackages that relocation-free test object as ELF32, and
links it into a Multiboot ELF using `tests/freestanding/bootstrap.S` and
`bootstrap.ld`. It then boots that image in QEMU. The bootstrap sets up x86-64 long mode, identity maps 1 GiB and supplies
the stack; it is external test infrastructure, not an +A implementation of a
boot protocol. QMP stops the guest and checks the result register, physical
memory and the executed HLT instruction. The test has a 15-second boot deadline
and fails if QEMU exits, times out, or the expected state is absent. It does not
validate physical machines, firmware portability or arbitrary boot protocols.

## Scope decision: machine-level interoperability

The QEMU test demonstrates Core's documented freestanding subset with external
boot infrastructure. Core does **not** provide arbitrary machine instructions,
direct hardware register or stack control, +A-initiated calls to externally
linked routines, a driver framework, or a complete boot protocol. The ELF
object and loader do not imply those capabilities. These are limitations, not
Core V2.0.0 release gates under the corrected scope. Any future ABI, hardware
interfaces or system abstractions may be considered in Foundation or Evolution;
none is implemented here. The prior proposed port-I/O QEMU test is consequently
not an acceptance requirement for this release.

## Release decision

The mandatory checks above passed on the stated Linux x86-64 environment.
Known limitations, test exclusions and compatibility boundaries remain explicit;
no release, tag or publication is implied.

The pinned Rust 1.97.0 archive was obtained via the GitHub mirror
`kelexine/rust-toolchain` and had SHA-256
`1cf17e4905b841d4c8e3f76467ac148d55fb3f54bf213c86f0d287a36471d904`,
matching the independently published `rust-bin.hash` in `baidxi/buildroot`.
It was installed outside the source tree; no toolchain binaries are vendored.
