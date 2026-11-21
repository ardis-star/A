use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::fmt;
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use crate::native::emit_assembly;

const RTLD_NOW: c_int = 2;
const RTLD_LOCAL: c_int = 0;

#[link(name = "dl")]
unsafe extern "C" {
    fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    fn dlclose(handle: *mut c_void) -> c_int;
    fn dlerror() -> *const c_char;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JitError(pub String);

impl fmt::Display for JitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for JitError {}

struct LoadedLibrary(*mut c_void);

impl Drop for LoadedLibrary {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                dlclose(self.0);
            }
        }
    }
}

fn dynamic_error(prefix: &str) -> JitError {
    let message = unsafe {
        let error = dlerror();
        if error.is_null() {
            "unknown dynamic loader error".to_owned()
        } else {
            CStr::from_ptr(error).to_string_lossy().into_owned()
        }
    };
    JitError(format!("{prefix}: {message}"))
}

fn temporary_library_path() -> Result<PathBuf, JitError> {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    for attempt in 0..32_u8 {
        let directory = std::env::temp_dir().join(format!(
            "ascension-jit-{}-{nonce}-{attempt}",
            std::process::id()
        ));
        match std::fs::create_dir(&directory) {
            Ok(()) => {
                if let Err(error) =
                    std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))
                {
                    let _ = std::fs::remove_dir(&directory);
                    return Err(JitError(format!(
                        "failed to secure JIT temp directory: {error}"
                    )));
                }
                return Ok(directory.join("compiled.so"));
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(JitError(format!(
                    "failed to reserve JIT temp directory: {error}"
                )));
            }
        }
    }
    Err(JitError(
        "could not reserve a unique JIT temp directory".into(),
    ))
}

fn compile_shared(assembly: &str, path: &std::path::Path) -> Result<(), JitError> {
    let mut child = Command::new("cc")
        .args([
            "-shared",
            "-fPIC",
            "-Wl,-z,relro,-z,now,-z,separate-code",
            "-Wl,-z,noexecstack",
            "-x",
            "assembler",
            "-",
            "-o",
        ])
        .arg(path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| JitError(format!("failed to start runtime compiler: {error}")))?;
    child
        .stdin
        .take()
        .ok_or_else(|| JitError("failed to open runtime compiler stdin".into()))?
        .write_all(assembly.as_bytes())
        .map_err(|error| JitError(format!("failed to pass code to runtime compiler: {error}")))?;
    let result = child
        .wait_with_output()
        .map_err(|error| JitError(format!("failed to wait for runtime compiler: {error}")))?;
    if !result.status.success() {
        return Err(JitError(format!(
            "JIT native compilation failed: {}",
            String::from_utf8_lossy(&result.stderr).trim()
        )));
    }
    Ok(())
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub fn run_jit(image: &[u8]) -> Result<i32, JitError> {
    let assembly = emit_assembly(image).map_err(|error| JitError(error.to_string()))?;
    let path = temporary_library_path()?;
    let result = (|| {
        compile_shared(&assembly, &path)?;
        let filename = CString::new(path.as_os_str().as_bytes())
            .map_err(|_| JitError("temporary shared object path contains NUL".into()))?;
        let handle = unsafe { dlopen(filename.as_ptr(), RTLD_NOW | RTLD_LOCAL) };
        if handle.is_null() {
            return Err(dynamic_error("failed to load JIT code"));
        }
        let library = LoadedLibrary(handle);
        let symbol = CString::new("main").expect("static symbol has no NUL");
        let entry = unsafe { dlsym(library.0, symbol.as_ptr()) };
        if entry.is_null() {
            return Err(dynamic_error("JIT entry symbol is missing"));
        }
        let function: unsafe extern "C" fn() -> i32 = unsafe { std::mem::transmute(entry) };
        let result = unsafe { function() };
        drop(library);
        Ok(result)
    })();
    let _ = std::fs::remove_file(&path);
    if let Some(directory) = path.parent() {
        let _ = std::fs::remove_dir(directory);
    }
    result
}

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
pub fn run_jit(_image: &[u8]) -> Result<i32, JitError> {
    Err(JitError(
        "the current JIT requires Linux x86-64 and a local cc toolchain".into(),
    ))
}
