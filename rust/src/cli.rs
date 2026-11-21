//! Command-line interface implementation for ASCENSION +A (`+a` / `aa`).

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crate::assemble;
use crate::avm::{Avm, RunResult};
use crate::jit::run_jit;
use crate::native::compile_executable;
use crate::verifier::{decode_image, verify_image};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn usage() -> &'static str {
    "usage: +a <source.pa> | +a run <source.pa|image.avm> | +a build <source.pa> [-o <output>] | +a check <source.pa> | +a --help | +a --version"
}

pub fn help_text() -> &'static str {
    "ASCENSION +A (+a) - The +A language toolchain

USAGE:
    +a <source.pa>                      Build, verify and execute directly
    +a run <source.pa|image.avm>        Explicitly execute (+A source or .avm bytecode)
    +a build <source.pa> [OPTIONS]      Build without executing
    +a check <source.pa>                Parse, assemble and verify without saving
    +a verify <image.avm>               Verify an existing .avm bytecode image
    +a inspect <image.avm>              Inspect bytecode structure and sections
    +a run-jit <image.avm>              Execute bytecode image using the JIT compiler
    +a compile-native <image> -o <bin>  AOT-compile bytecode image to native executable
    +a --version                        Show version information
    +a --help                           Show this help message

OPTIONS:
    -o, --output <path>     Specify custom output file path
    --build-dir <dir>       Directory for generated files (default: ./build)
    --native, -n            Build or execute as native x86-64 binary
    --jit                   Execute with JIT compiler instead of interpreter

FILE EXTENSIONS:
    .pa     +A source file (official extension)
    .avm    ASCENSION bytecode image
"
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    PlusA,
    LegacyA,
}

pub fn validate_source_extension(path: &Path) -> Result<SourceKind, String> {
    match path.extension().and_then(|s| s.to_str()) {
        Some("pa") => Ok(SourceKind::PlusA),
        Some("a") => Ok(SourceKind::LegacyA),
        Some("ma") => Err(format!(
            "error: unsupported file extension '.ma' for '{}': '-A' is not supported; only '+A' (.pa) source files are supported",
            path.display()
        )),
        Some(ext) => Err(format!(
            "error: invalid file extension '.{ext}' for '{}': expected a '+A' source file with '.pa' extension",
            path.display()
        )),
        None => Err(format!(
            "error: missing file extension for '{}': expected a '+A' source file with '.pa' extension",
            path.display()
        )),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetKind {
    Source(SourceKind),
    Bytecode,
}

pub fn validate_run_target(path: &Path) -> Result<TargetKind, String> {
    match path.extension().and_then(|s| s.to_str()) {
        Some("pa") => Ok(TargetKind::Source(SourceKind::PlusA)),
        Some("a") => Ok(TargetKind::Source(SourceKind::LegacyA)),
        Some("avm") => Ok(TargetKind::Bytecode),
        Some("ma") => Err(format!(
            "error: unsupported file extension '.ma' for '{}': '-A' is not supported; only '+A' (.pa) source files are supported",
            path.display()
        )),
        Some(ext) => Err(format!(
            "error: invalid file extension '.{ext}' for '{}': expected a '+A' source file ('.pa') or bytecode image ('.avm')",
            path.display()
        )),
        None => Err(format!(
            "error: missing file extension for '{}': expected a '+A' source file ('.pa') or bytecode image ('.avm')",
            path.display()
        )),
    }
}

pub fn format_assembly_error(path: &Path, error: &crate::AssemblyError) -> String {
    let msg = error.to_string();
    if msg.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        format!("{}:{msg}", path.display())
    } else {
        format!("{}: error: {msg}", path.display())
    }
}

pub fn execute_avm(bytecode: &[u8]) -> Result<(), String> {
    let mut machine = Avm::load(bytecode).map_err(|err| err.to_string())?;
    match machine
        .run()
        .map_err(|err| format!("error: runtime fault: {err}"))?
    {
        RunResult::Returned => {
            println!("returned");
            Ok(())
        }
        RunResult::Stopped(status) => {
            println!("stopped: {status}");
            Ok(())
        }
    }
}

pub fn run_args<I, T>(args: I) -> Result<(), String>
where
    I: IntoIterator<Item = T>,
    T: Into<String>,
{
    let args: Vec<String> = args.into_iter().map(Into::into).collect();
    if args.is_empty() {
        return Err(format!(
            "{}\nerror: no command or source file provided",
            usage()
        ));
    }

    let first = &args[0];

    // Version
    if matches!(first.as_str(), "--version" | "-V" | "version") {
        println!("+a {VERSION}");
        return Ok(());
    }

    // Help
    if matches!(first.as_str(), "--help" | "-h" | "help") {
        println!("{}", help_text());
        return Ok(());
    }

    // Subcommands
    match first.as_str() {
        "check" => handle_check(&args[1..]),
        "build" => handle_build(&args[1..]),
        "run" => handle_run(&args[1..]),
        "verify" => handle_verify(&args[1..]),
        "inspect" => handle_inspect(&args[1..]),
        "run-jit" => handle_run_jit(&args[1..]),
        "compile-native" => handle_compile_native(&args[1..]),
        _ if first.starts_with('-') => {
            Err(format!("{}\nerror: unrecognized option '{first}'", usage()))
        }
        _ => handle_direct_or_legacy(&args),
    }
}

fn handle_check(args: &[String]) -> Result<(), String> {
    if args.is_empty() {
        return Err(format!(
            "{}\nerror: source path is required for 'check'",
            usage()
        ));
    }
    let mut source: Option<PathBuf> = None;
    for arg in args {
        match arg.as_str() {
            "-h" | "--help" => {
                println!("usage: +a check <source.pa>");
                return Ok(());
            }
            _ if arg.starts_with('-') => {
                return Err(format!("error: unrecognized option '{arg}' for 'check'"));
            }
            _ => {
                if source.replace(PathBuf::from(arg)).is_some() {
                    return Err("error: only one source file is accepted for 'check'".to_string());
                }
            }
        }
    }
    let source_path =
        source.ok_or_else(|| "error: source path is required for 'check'".to_string())?;
    validate_source_extension(&source_path)?;

    let text = fs::read_to_string(&source_path)
        .map_err(|err| format!("error: cannot read '{}': {err}", source_path.display()))?
        .replace("\r\n", "\n");
    let image = assemble(&text).map_err(|err| format_assembly_error(&source_path, &err))?;
    verify_image(&image)
        .map_err(|err| format!("{}: verification failed: {err}", source_path.display()))?;

    println!("checked: {}", source_path.display());
    Ok(())
}

fn handle_build(args: &[String]) -> Result<(), String> {
    if args.is_empty() {
        return Err(format!(
            "{}\nerror: source path is required for 'build'",
            usage()
        ));
    }
    let mut source: Option<PathBuf> = None;
    let mut output: Option<PathBuf> = None;
    let mut build_dir: Option<PathBuf> = None;
    let mut native = false;
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        if arg == "-o" || arg == "--output" {
            i += 1;
            if i >= args.len() {
                return Err("error: output path is required after -o/--output".to_string());
            }
            output = Some(PathBuf::from(&args[i]));
        } else if let Some(val) = arg.strip_prefix("--output=") {
            output = Some(PathBuf::from(val));
        } else if arg == "--build-dir" {
            i += 1;
            if i >= args.len() {
                return Err("error: directory path is required after --build-dir".to_string());
            }
            build_dir = Some(PathBuf::from(&args[i]));
        } else if let Some(val) = arg.strip_prefix("--build-dir=") {
            build_dir = Some(PathBuf::from(val));
        } else if arg == "--native" || arg == "-n" {
            native = true;
        } else if arg == "-h" || arg == "--help" {
            println!("usage: +a build <source.pa> [-o <output>] [--build-dir <dir>] [--native]");
            return Ok(());
        } else if arg.starts_with('-') {
            return Err(format!("error: unrecognized option '{arg}' for 'build'"));
        } else if source.replace(PathBuf::from(arg)).is_some() {
            return Err("error: only one source file is accepted for 'build'".to_string());
        }
        i += 1;
    }
    let source_path =
        source.ok_or_else(|| "error: source path is required for 'build'".to_string())?;
    validate_source_extension(&source_path)?;

    let text = fs::read_to_string(&source_path)
        .map_err(|err| format!("error: cannot read '{}': {err}", source_path.display()))?
        .replace("\r\n", "\n");
    let image = assemble(&text).map_err(|err| format_assembly_error(&source_path, &err))?;
    verify_image(&image)
        .map_err(|err| format!("{}: verification failed: {err}", source_path.display()))?;

    let default_build = PathBuf::from("build");
    let bdir = build_dir.as_ref().unwrap_or(&default_build);
    let stem = source_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("output");

    if native {
        let out_path = output.unwrap_or_else(|| bdir.join("native").join(stem));
        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent).map_err(|err| {
                format!(
                    "error: failed to create build directory '{}': {err}",
                    parent.display()
                )
            })?;
        }
        compile_executable(&image, &out_path)
            .map_err(|err| format!("native compile error: {err}"))?;
        println!("built: {}", out_path.display());
    } else {
        let out_path = output.unwrap_or_else(|| bdir.join("bytecode").join(format!("{stem}.avm")));
        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent).map_err(|err| {
                format!(
                    "error: failed to create build directory '{}': {err}",
                    parent.display()
                )
            })?;
        }
        fs::write(&out_path, &image)
            .map_err(|err| format!("error: failed to write '{}': {err}", out_path.display()))?;
        println!("built: {}", out_path.display());
    }
    Ok(())
}

fn handle_run(args: &[String]) -> Result<(), String> {
    if args.is_empty() {
        return Err(format!(
            "{}\nerror: target path is required for 'run'",
            usage()
        ));
    }
    let mut target: Option<PathBuf> = None;
    let mut build_dir: Option<PathBuf> = None;
    let mut jit = false;
    let mut native = false;
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        if arg == "--jit" {
            jit = true;
        } else if arg == "--native" || arg == "-n" {
            native = true;
        } else if arg == "--build-dir" {
            i += 1;
            if i >= args.len() {
                return Err("error: directory path is required after --build-dir".to_string());
            }
            build_dir = Some(PathBuf::from(&args[i]));
        } else if let Some(val) = arg.strip_prefix("--build-dir=") {
            build_dir = Some(PathBuf::from(val));
        } else if arg == "-h" || arg == "--help" {
            println!("usage: +a run <source.pa|image.avm> [--jit] [--native] [--build-dir <dir>]");
            return Ok(());
        } else if arg.starts_with('-') {
            return Err(format!("error: unrecognized option '{arg}' for 'run'"));
        } else if target.replace(PathBuf::from(arg)).is_some() {
            return Err("error: unexpected extra argument for 'run'".to_string());
        }
        i += 1;
    }
    let target_path =
        target.ok_or_else(|| "error: target path is required for 'run'".to_string())?;
    let target_kind = validate_run_target(&target_path)?;

    match target_kind {
        TargetKind::Source(_) => {
            let text = fs::read_to_string(&target_path)
                .map_err(|err| format!("error: cannot read '{}': {err}", target_path.display()))?
                .replace("\r\n", "\n");
            let image = assemble(&text).map_err(|err| format_assembly_error(&target_path, &err))?;
            verify_image(&image)
                .map_err(|err| format!("{}: verification failed: {err}", target_path.display()))?;

            let default_build = PathBuf::from("build");
            let bdir = build_dir.as_ref().unwrap_or(&default_build);
            let stem = target_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("output");
            let bytecode_path = bdir.join("bytecode").join(format!("{stem}.avm"));
            if let Some(parent) = bytecode_path.parent() {
                fs::create_dir_all(parent).map_err(|err| {
                    format!(
                        "error: failed to create build directory '{}': {err}",
                        parent.display()
                    )
                })?;
            }
            fs::write(&bytecode_path, &image).map_err(|err| {
                format!(
                    "error: failed to write bytecode to '{}': {err}",
                    bytecode_path.display()
                )
            })?;

            if jit {
                let result = run_jit(&image).map_err(|err| format!("jit error: {err}"))?;
                println!("jit returned: {result}");
                Ok(())
            } else if native {
                let native_path = bdir.join("native").join(stem);
                if let Some(parent) = native_path.parent() {
                    fs::create_dir_all(parent).map_err(|err| {
                        format!(
                            "error: failed to create build directory '{}': {err}",
                            parent.display()
                        )
                    })?;
                }
                compile_executable(&image, &native_path)
                    .map_err(|err| format!("native compile error: {err}"))?;
                let status = std::process::Command::new(&native_path)
                    .status()
                    .map_err(|err| {
                        format!("failed to execute '{}': {err}", native_path.display())
                    })?;
                if let Some(code) = status.code() {
                    println!("stopped: {code}");
                }
                Ok(())
            } else {
                execute_avm(&image)
            }
        }
        TargetKind::Bytecode => {
            let image = fs::read(&target_path)
                .map_err(|err| format!("error: cannot read '{}': {err}", target_path.display()))?;
            if jit {
                let result = run_jit(&image).map_err(|err| format!("jit error: {err}"))?;
                println!("jit returned: {result}");
                Ok(())
            } else {
                let mut machine = Avm::load(&image).map_err(|err| err.to_string())?;
                match machine
                    .run()
                    .map_err(|err| format!("error: runtime fault: {err}"))?
                {
                    RunResult::Returned => println!("returned"),
                    RunResult::Stopped(status) => println!("stopped: {status}"),
                }
                Ok(())
            }
        }
    }
}

fn handle_verify(args: &[String]) -> Result<(), String> {
    let mut arguments = args.iter();
    let image_path = PathBuf::from(
        arguments
            .next()
            .ok_or_else(|| format!("{}\nerror: image path is required", usage()))?,
    );
    if arguments.next().is_some() {
        return Err(format!("{}\nerror: unexpected extra argument", usage()));
    }
    let image =
        fs::read(&image_path).map_err(|error| format!("{}: {}", image_path.display(), error))?;
    let verified = verify_image(&image).map_err(|error| error.to_string())?;
    println!(
        "verified: {} function(s), entry {:?}",
        verified.functions.len(),
        verified.entry_function
    );
    Ok(())
}

fn handle_inspect(args: &[String]) -> Result<(), String> {
    let mut arguments = args.iter();
    let image_path = PathBuf::from(
        arguments
            .next()
            .ok_or_else(|| format!("{}\nerror: image path is required", usage()))?,
    );
    if arguments.next().is_some() {
        return Err(format!("{}\nerror: unexpected extra argument", usage()));
    }
    let image =
        fs::read(&image_path).map_err(|error| format!("{}: {}", image_path.display(), error))?;
    let decoded = decode_image(&image).map_err(|error| error.to_string())?;
    println!(
        ".avm v1.{}: {} function(s), entry {:?}, {} unresolved control target(s)",
        decoded.minor_version,
        decoded.functions.len(),
        decoded.entry_function,
        decoded.unresolved_control_targets.len()
    );
    for function in decoded.functions {
        println!(
            "function {}: {} byte(s), {} instruction(s)",
            function.id,
            function.code_length,
            function.instructions.len()
        );
    }
    Ok(())
}

fn handle_run_jit(args: &[String]) -> Result<(), String> {
    let mut arguments = args.iter();
    let image_path = PathBuf::from(
        arguments
            .next()
            .ok_or_else(|| format!("{}\nerror: image path is required", usage()))?,
    );
    if arguments.next().is_some() {
        return Err(format!("{}\nerror: unexpected extra argument", usage()));
    }
    let image =
        fs::read(&image_path).map_err(|error| format!("{}: {}", image_path.display(), error))?;
    let result = run_jit(&image).map_err(|error| error.to_string())?;
    println!("jit returned: {result}");
    Ok(())
}

fn handle_compile_native(args: &[String]) -> Result<(), String> {
    let mut arguments = args.iter();
    let image_path = PathBuf::from(
        arguments
            .next()
            .ok_or_else(|| format!("{}\nerror: image path is required", usage()))?,
    );
    let option = arguments
        .next()
        .ok_or_else(|| format!("{}\nerror: --output is required", usage()))?;
    if option != "-o" && option != "--output" {
        return Err(format!("{}\nerror: expected -o/--output", usage()));
    }
    let output_path = PathBuf::from(
        arguments
            .next()
            .ok_or_else(|| format!("{}\nerror: executable path is required", usage()))?,
    );
    if arguments.next().is_some() {
        return Err(format!("{}\nerror: unexpected extra argument", usage()));
    }
    let image =
        fs::read(&image_path).map_err(|error| format!("{}: {}", image_path.display(), error))?;
    compile_executable(&image, &output_path).map_err(|error| error.to_string())
}

fn handle_direct_or_legacy(args: &[String]) -> Result<(), String> {
    let source_path = PathBuf::from(&args[0]);
    let mut output: Option<PathBuf> = None;
    let mut build_dir: Option<PathBuf> = None;
    let mut native = false;
    let mut jit = false;
    let mut positional_only = false;
    let mut i = 1;

    while i < args.len() {
        let arg = &args[i];
        if positional_only {
            return Err(format!(
                "{}\nerror: only one source file is accepted",
                usage()
            ));
        }
        match arg.as_str() {
            "-h" | "--help" => {
                println!("{}", help_text());
                return Ok(());
            }
            "--" => positional_only = true,
            "-o" | "--output" => {
                i += 1;
                if i >= args.len() {
                    return Err(format!("{}\nerror: output path is required", usage()));
                }
                output = Some(PathBuf::from(&args[i]));
            }
            _ if arg.starts_with("--output=") => output = Some(PathBuf::from(&arg[9..])),
            "--build-dir" => {
                i += 1;
                if i >= args.len() {
                    return Err(format!("{}\nerror: directory path is required", usage()));
                }
                build_dir = Some(PathBuf::from(&args[i]));
            }
            _ if arg.starts_with("--build-dir=") => build_dir = Some(PathBuf::from(&arg[12..])),
            "--native" | "-n" => native = true,
            "--jit" => jit = true,
            _ if arg.starts_with('-') => {
                return Err(format!(
                    "{}\nerror: unrecognized argument {:?}",
                    usage(),
                    arg
                ));
            }
            _ => {
                return Err(format!(
                    "{}\nerror: only one source file is accepted",
                    usage()
                ));
            }
        }
        i += 1;
    }

    validate_source_extension(&source_path)?;

    let text = fs::read_to_string(&source_path)
        .map_err(|error| format!("{}: {}", source_path.display(), error))?
        .replace("\r\n", "\n");
    let image = assemble(&text).map_err(|error| format_assembly_error(&source_path, &error))?;

    // If explicit output was specified, this is legacy assembly mode: aa <source> -o <output>
    if let Some(out_path) = output {
        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent).map_err(|err| {
                format!(
                    "error: failed to create directory '{}': {err}",
                    parent.display()
                )
            })?;
        }
        return fs::write(&out_path, &image)
            .map_err(|error| format!("{}: {}", out_path.display(), error));
    }

    // Direct execution: +a program.pa
    verify_image(&image)
        .map_err(|error| format!("{}: verification failed: {error}", source_path.display()))?;

    // Store generated bytecode in build directory
    let default_build = PathBuf::from("build");
    let bdir = build_dir.as_ref().unwrap_or(&default_build);
    let stem = source_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("output");
    let bytecode_path = bdir.join("bytecode").join(format!("{stem}.avm"));
    if let Some(parent) = bytecode_path.parent() {
        fs::create_dir_all(parent).map_err(|err| {
            format!(
                "error: failed to create build directory '{}': {err}",
                parent.display()
            )
        })?;
    }
    fs::write(&bytecode_path, &image).map_err(|err| {
        format!(
            "error: failed to write bytecode to '{}': {err}",
            bytecode_path.display()
        )
    })?;

    if jit {
        let result = run_jit(&image).map_err(|err| format!("jit error: {err}"))?;
        println!("jit returned: {result}");
        Ok(())
    } else if native {
        let native_path = bdir.join("native").join(stem);
        if let Some(parent) = native_path.parent() {
            fs::create_dir_all(parent).map_err(|err| {
                format!(
                    "error: failed to create build directory '{}': {err}",
                    parent.display()
                )
            })?;
        }
        compile_executable(&image, &native_path)
            .map_err(|err| format!("native compile error: {err}"))?;
        let status = std::process::Command::new(&native_path)
            .status()
            .map_err(|err| format!("failed to execute '{}': {err}", native_path.display()))?;
        if let Some(code) = status.code() {
            println!("stopped: {code}");
        }
        Ok(())
    } else {
        execute_avm(&image)
    }
}

pub fn run() -> Result<(), String> {
    run_args(env::args().skip(1))
}

pub fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{}", error);
            ExitCode::from(2)
        }
    }
}
