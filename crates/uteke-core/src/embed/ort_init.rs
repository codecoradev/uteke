//! Runtime CPU feature detection and ORT library resolution.
//!
//! Solves SIGILL (Exit Code 132) on CPUs without AVX2 (e.g., Celeron J4125/N4020)
//! by dynamically selecting the correct ONNX Runtime shared library at startup.
//!
//! Resolution order:
//! 1. `ORT_LIB_PATH` env var — explicit override (for testing / custom setups).
//!    Names the library file; a directory holding the library is accepted too.
//! 2. If AVX2 is detected → `<exe_dir>/libonnxruntime.so` (AVX2 build, ships in standard bundle).
//!    If not found, falls through to step 4 (does NOT error immediately).
//! 3. If AVX2 is NOT detected → `<exe_dir>/ort-legacy/libonnxruntime.so` (SSE4.2 build, sidecar).
//! 4. System lib paths: `/usr/local/lib`, `/usr/lib`, `/lib` (Docker / package installs).
//! 5. If no library found → return error with all searched locations.

use std::path::PathBuf;

/// Name of the legacy ORT sidecar directory (placed next to the binary).
const LEGACY_ORT_DIR: &str = "ort-legacy";

/// ORT shared library filename per platform.
#[cfg(any(target_os = "linux", target_os = "android"))]
const ORT_LIB_NAME: &str = "libonnxruntime.so";
#[cfg(any(target_os = "macos", target_os = "ios"))]
const ORT_LIB_NAME: &str = "libonnxruntime.dylib";
#[cfg(target_os = "windows")]
const ORT_LIB_NAME: &str = "onnxruntime.dll";

/// Result of CPU feature detection and ORT library resolution.
pub struct OrtLibInfo {
    /// Path to the resolved ORT shared library.
    pub lib_path: PathBuf,
    /// Whether AVX2 was detected on this CPU.
    pub has_avx2: bool,
}

impl std::fmt::Debug for OrtLibInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OrtLibInfo")
            .field("lib_path", &self.lib_path)
            .field("has_avx2", &self.has_avx2)
            .finish()
    }
}

/// Detect whether the current CPU supports AVX2.
///
/// Returns `false` on non-x86_64 architectures (ARM, etc.) since those
/// don't use AVX2 at all — the standard ORT build will work fine.
#[inline]
pub fn has_avx2() -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        // SAFETY: `is_x86_feature_detected!` is a compile-time macro that
        // generates a `cpuid` instruction. It is safe to call at any time.
        std::is_x86_feature_detected!("avx2")
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        // Non-x86_64 (ARM, RISC-V, etc.) — no AVX2 concept.
        // Use the standard library; the x86-specific SIGILL issue doesn't apply.
        true
    }
}

/// Resolve an explicit `ORT_LIB_PATH` value to the library file to load.
///
/// The value should name the library file. A directory is accepted too, as long
/// as the platform library name is inside it: the Homebrew shims exported the
/// `libexec` directory from v0.19.0 on, and `dlopen` fails on a directory, so
/// every such install silently ran without embeddings (#1459).
fn resolve_explicit_ort_path(path: &std::path::Path) -> Result<PathBuf, String> {
    if path.is_dir() {
        let inside = path.join(ORT_LIB_NAME);
        if inside.exists() {
            tracing::warn!(
                "ORT_LIB_PATH '{}' is a directory; using {}. Set it to the library file.",
                path.display(),
                inside.display()
            );
            return Ok(inside);
        }
        return Err(format!(
            "ORT_LIB_PATH '{}' is a directory with no {ORT_LIB_NAME} inside; \
             set it to the library file",
            path.display()
        ));
    }
    if path.exists() {
        return Ok(path.to_path_buf());
    }
    Err(format!(
        "ORT_LIB_PATH set to '{}' but file does not exist",
        path.display()
    ))
}

/// Resolve the path to the ONNX Runtime shared library.
///
/// Search order:
/// 1. `ORT_LIB_PATH` environment variable (explicit override).
/// 2. AVX2 detected → `<exe_dir>/libonnxruntime.so` (standard bundle).
///    If not found, falls through to step 4.
/// 3. No AVX2 → `<exe_dir>/ort-legacy/libonnxruntime.so` (legacy sidecar).
/// 4. Unix system paths: `/usr/local/lib`, `/usr/lib`, `/lib`.
/// 5. Error with all searched locations.
///
/// # Errors
///
/// Returns a descriptive error if no ORT library can be found.
pub fn resolve_ort_lib() -> Result<OrtLibInfo, String> {
    // 1. Explicit env var override
    if let Ok(env_path) = std::env::var("ORT_LIB_PATH") {
        let lib_path = resolve_explicit_ort_path(&PathBuf::from(env_path))?;
        tracing::info!(
            "ORT: using explicit library path from ORT_LIB_PATH: {}",
            lib_path.display()
        );
        return Ok(OrtLibInfo {
            lib_path,
            has_avx2: has_avx2(),
        });
    }

    let exe_dir = exe_dir()?;
    let avx2 = has_avx2();

    // 2. AVX2 CPU → try standard library next to binary
    if avx2 {
        let standard_path = exe_dir.join(ORT_LIB_NAME);
        if standard_path.exists() {
            tracing::info!(
                "ORT: AVX2 detected, using standard library: {}",
                standard_path.display()
            );
            return Ok(OrtLibInfo {
                lib_path: standard_path,
                has_avx2: true,
            });
        }
        // Standard lib not found in exe_dir — fall through to system paths.
        // This is common in Docker where binary is in /usr/local/bin/ but .so
        // is in /usr/local/lib/.
        tracing::debug!(
            "ORT: AVX2 detected but no standard library at {}. \
             Falling through to system path search.",
            standard_path.display()
        );
    }

    // 3. No AVX2 → look for legacy sidecar
    let legacy_path = exe_dir.join(LEGACY_ORT_DIR).join(ORT_LIB_NAME);
    if !avx2 && legacy_path.exists() {
        tracing::info!(
            "ORT: No AVX2 detected, using legacy (SSE4.2) library: {}",
            legacy_path.display()
        );
        return Ok(OrtLibInfo {
            lib_path: legacy_path,
            has_avx2: false,
        });
    }

    // 4. Docker/system fallback: check standard system library paths.
    // In Docker containers, binaries are in /usr/local/bin/ but .so is in /usr/local/lib/.
    // On Windows, system paths are not applicable (DLLs are loaded from exe dir or PATH).
    #[cfg(unix)]
    {
        let system_paths: &[&str] = &["/usr/local/lib", "/usr/lib", "/lib"];
        for dir in system_paths {
            let system_path = std::path::Path::new(dir).join(ORT_LIB_NAME);
            if system_path.exists() {
                tracing::info!("ORT: Using system library at {}", system_path.display());
                return Ok(OrtLibInfo {
                    lib_path: system_path,
                    has_avx2: avx2,
                });
            }
        }
    }

    // 5. Python site-packages fallback — many users have onnxruntime via pip.
    // This covers: ~/.local/lib/python*/site-packages/onnxruntime/capi/
    // Also checks pyke cache: ~/.cache/ort.pyke.io/
    if let Some(path) = find_python_or_conda_lib() {
        tracing::info!(
            "ORT: Using library from Python/conda install: {}",
            path.display()
        );
        return Ok(OrtLibInfo {
            lib_path: path,
            has_avx2: avx2,
        });
    }

    // 6. No library found anywhere — build a descriptive error.
    if avx2 {
        Err(format!(
            "ONNX Runtime library not found. Searched:\n\
             - {} (exe directory)\n\
             - /usr/local/lib, /usr/lib, /lib (system paths)\n\
             - Python site-packages (~/.local/lib/python*/site-packages/onnxruntime/capi/)\n\
             - pyke cache (~/.cache/ort.pyke.io/)\n\
             Set ORT_LIB_PATH to the library file, or use the standard release bundle.",
            exe_dir.join(ORT_LIB_NAME).display()
        ))
    } else {
        Err(format!(
            "No AVX2 support and legacy ORT library not found at '{}'. \
             Use the 'legacy' release bundle which includes the ort-legacy/ sidecar.",
            legacy_path.display()
        ))
    }
}

/// Get the directory containing the running executable.
fn exe_dir() -> Result<PathBuf, String> {
    let exe =
        std::env::current_exe().map_err(|e| format!("failed to determine executable path: {e}"))?;
    exe.parent()
        .map(|p| p.to_path_buf())
        .ok_or_else(|| "executable path has no parent directory".into())
}

/// Search for ONNX Runtime library in Python and conda installations.
///
/// Covers:
/// - `~/.local/lib/python*/site-packages/onnxruntime/capi/` (pip user install)
/// - `/usr/lib/python*/site-packages/onnxruntime/capi/` (pip system install)
/// - `~/.cache/ort.pyke.io/` (pyke cache, nested dfbin dirs)
/// - Conda: `~/miniconda3/lib/`, `~/anaconda3/lib/` (conda install)
///
/// Handles versioned library names: `libonnxruntime.so.1.25.0`, etc.
///
/// Returns the first match found, or `None`.
fn find_python_or_conda_lib() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;

    // Collect all candidate directories.
    let mut candidates: Vec<PathBuf> = Vec::new();

    // 1. pip user site-packages: ~/.local/lib/python*/site-packages/onnxruntime/capi/
    let pip_user_base = PathBuf::from(&home).join(".local/lib");
    if let Ok(entries) = std::fs::read_dir(&pip_user_base) {
        for entry in entries.flatten() {
            let py_dir = entry.path().join("site-packages/onnxruntime/capi");
            if py_dir.is_dir() {
                candidates.push(py_dir);
            }
        }
    }

    // 2. pip system site-packages: /usr/lib/python*/site-packages/onnxruntime/capi/
    //    and /usr/local/lib/python*/site-packages/onnxruntime/capi/
    for sys_base in ["/usr/lib", "/usr/local/lib"] {
        if let Ok(entries) = std::fs::read_dir(sys_base) {
            for entry in entries.flatten() {
                let py_dir = entry.path().join("site-packages/onnxruntime/capi");
                if py_dir.is_dir() {
                    candidates.push(py_dir);
                }
            }
        }
    }

    // 3. pyke cache: ~/.cache/ort.pyke.io/ (nested dfbin dirs with hash names)
    let pyke_base = PathBuf::from(&home).join(".cache/ort.pyke.io");
    if pyke_base.is_dir() {
        collect_pyke_dirs(&pyke_base, &mut candidates, 0);
    }

    // 4. Conda: ~/miniconda3/lib/, ~/anaconda3/lib/, ~/miniforge3/lib/
    for conda in ["miniconda3", "anaconda3", "miniforge3"] {
        candidates.push(PathBuf::from(&home).join(conda).join("lib"));
    }

    // Search each candidate for the ORT library (exact name or versioned variant).
    for dir in &candidates {
        if let Some(path) = find_ort_in_dir(dir) {
            return Some(path);
        }
    }

    None
}

/// Recursively collect directories from pyke cache (max depth 3).
fn collect_pyke_dirs(base: &PathBuf, candidates: &mut Vec<PathBuf>, depth: usize) {
    if depth > 3 {
        return;
    }
    if let Ok(entries) = std::fs::read_dir(base) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                // Check if this dir contains the ORT lib directly.
                if find_ort_in_dir(&path).is_some() {
                    candidates.push(path);
                } else {
                    // Recurse deeper.
                    collect_pyke_dirs(&path, candidates, depth + 1);
                }
            }
        }
    }
}

/// Look for the ORT library in a specific directory.
///
/// Tries exact filename first (`libonnxruntime.so`), then versioned
/// variants (`libonnxruntime.so.*`).
fn find_ort_in_dir(dir: &PathBuf) -> Option<PathBuf> {
    // Exact match.
    let exact = dir.join(ORT_LIB_NAME);
    if exact.exists() {
        return Some(exact);
    }

    // Versioned variant: libonnxruntime.so.1.25.0, etc.
    // Only on Unix (.so suffix).
    #[cfg(unix)]
    {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                // Match: libonnxruntime.so.<anything>
                if name_str.starts_with("libonnxruntime.so.") && entry.path().is_file() {
                    return Some(entry.path());
                }
            }
        }
    }

    None
}

/// The process-wide ORT environment. Since `ort` 2.0.0-rc.14 a session is built
/// from an [`Environment`](ort::environment::Environment), and only one may
/// exist per process, so it lives here for the life of the process.
static ENVIRONMENT: std::sync::OnceLock<ort::environment::Environment> = std::sync::OnceLock::new();

/// The environment created by [`init_ort_environment`], or `None` before it
/// has run (or when it failed).
pub fn environment() -> Option<&'static ort::environment::Environment> {
    ENVIRONMENT.get()
}

/// Initialize the ORT environment using the resolved library path.
///
/// This must be called **once** before any `ort::Session` is created; the
/// environment it builds is then available from [`environment`].
/// Returns the `OrtLibInfo` on success so callers can log which variant is in use.
///
/// # Errors
///
/// Returns an error if:
/// - No suitable ORT library can be found (`resolve_ort_lib` failure).
/// - The library fails to load (`ort::init_from` failure, e.g. version mismatch).
pub fn init_ort_environment() -> Result<OrtLibInfo, String> {
    let info = resolve_ort_lib()?;

    let environment = ort::init_from(&info.lib_path)
        .map_err(|e| {
            format!(
                "Failed to load ONNX Runtime from '{}': {e}. \
             Ensure the library version matches ort crate 2.0.0-rc.14.",
                info.lib_path.display()
            )
        })?
        .build()
        .map_err(|e| format!("Failed to create the ONNX Runtime environment: {e}"))?;
    // Only the first call can get here: a second `build` is refused by ort.
    let _ = ENVIRONMENT.set(environment);

    tracing::info!(
        "ORT environment initialized successfully (AVX2={})",
        info.has_avx2
    );

    Ok(info)
}

#[allow(unsafe_code)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn has_avx2_returns_bool() {
        // Just verify it doesn't panic — the actual value depends on the test machine.
        let _avx2 = has_avx2();
    }

    /// env-var tests share the process-global ORT_LIB_PATH, so they run
    /// sequentially INSIDE one #[test] — parallel #[test]s mutating the same
    /// env var race each other (valid path overwritten by the invalid one
    /// mid-resolve), which flaked CI nondeterministically.
    #[test]
    fn resolve_ort_lib_env_var_override() {
        // 1) Valid path → Ok, and the resolved path equals the override
        let tmp = std::env::temp_dir().join("test_ort_lib_resolve.so");
        std::fs::write(&tmp, b"fake").unwrap();

        // SAFETY: single-threaded test section; no other test touches this var
        // while we hold it (see note above on why these cases are merged).
        unsafe {
            std::env::set_var("ORT_LIB_PATH", &tmp);
        }
        let result = resolve_ort_lib();
        unsafe {
            std::env::remove_var("ORT_LIB_PATH");
        }
        std::fs::remove_file(&tmp).ok();

        assert!(result.is_ok());
        assert_eq!(result.unwrap().lib_path, tmp);

        // 2) Nonexistent path → Err mentioning the missing file
        // SAFETY: same single-threaded section.
        unsafe {
            std::env::set_var("ORT_LIB_PATH", "/nonexistent/path/libonnxruntime.so");
        }
        let result = resolve_ort_lib();
        unsafe {
            std::env::remove_var("ORT_LIB_PATH");
        }

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("does not exist"));
    }

    /// Unique empty temp dir for one test (the file name keeps parallel tests apart).
    fn scratch_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("uteke_ort_explicit_{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn explicit_path_to_a_file_is_used_as_is() {
        let dir = scratch_dir("file");
        let lib = dir.join("custom-name.bin");
        std::fs::write(&lib, b"fake").unwrap();

        assert_eq!(resolve_explicit_ort_path(&lib).unwrap(), lib);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The Homebrew shims exported ORT_LIB_PATH=<libexec> (a directory) from
    /// v0.19.0 on; dlopen fails on a directory, so every install silently ran
    /// without embeddings (#1459).
    #[test]
    fn explicit_path_to_a_directory_finds_the_library_inside() {
        let dir = scratch_dir("dir_with_lib");
        let lib = dir.join(ORT_LIB_NAME);
        std::fs::write(&lib, b"fake").unwrap();

        assert_eq!(resolve_explicit_ort_path(&dir).unwrap(), lib);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn explicit_path_to_a_directory_without_the_library_is_an_error() {
        let dir = scratch_dir("dir_without_lib");

        let err = resolve_explicit_ort_path(&dir).unwrap_err();
        assert_eq!(
            err,
            format!(
                "ORT_LIB_PATH '{}' is a directory with no {ORT_LIB_NAME} inside; \
                 set it to the library file",
                dir.display()
            )
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn explicit_path_that_does_not_exist_is_an_error() {
        let missing = PathBuf::from("/nonexistent/path/libonnxruntime.so");

        let err = resolve_explicit_ort_path(&missing).unwrap_err();
        assert_eq!(
            err,
            "ORT_LIB_PATH set to '/nonexistent/path/libonnxruntime.so' but file does not exist"
        );
    }

    #[test]
    fn exe_dir_returns_parent() {
        let dir = exe_dir().unwrap();
        assert!(dir.exists(), "exe dir should exist: {}", dir.display());
    }

    #[test]
    fn find_ort_in_dir_finds_exact_match() {
        let tmp = std::env::temp_dir();
        let dir = tmp.join("test_ort_exact");
        std::fs::create_dir_all(&dir).unwrap();
        let lib = dir.join(ORT_LIB_NAME);
        std::fs::write(&lib, b"fake").unwrap();

        let result = find_ort_in_dir(&dir);
        assert!(result.is_some());
        assert_eq!(result.unwrap(), lib);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn find_ort_in_dir_finds_versioned_variant() {
        let tmp = std::env::temp_dir();
        let dir = tmp.join("test_ort_versioned");
        std::fs::create_dir_all(&dir).unwrap();
        let lib = dir.join("libonnxruntime.so.1.25.0");
        std::fs::write(&lib, b"fake").unwrap();

        let result = find_ort_in_dir(&dir);
        assert!(result.is_some());
        assert_eq!(result.unwrap(), lib);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn find_ort_in_dir_returns_none_when_empty() {
        let tmp = std::env::temp_dir();
        let dir = tmp.join("test_ort_empty");
        std::fs::create_dir_all(&dir).unwrap();

        let result = find_ort_in_dir(&dir);
        assert!(result.is_none());

        std::fs::remove_dir_all(&dir).ok();
    }
}
