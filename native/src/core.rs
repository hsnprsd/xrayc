//! Safe ownership of the embedded Xray C ABI. The Go runtime must stay loaded
//! until process exit; unloading a Go shared library is not supported.
use libloading::Library;
use serde_json::Value;
use std::{
    ffi::{c_char, CStr, CString},
    io,
    mem::ManuallyDrop,
    path::{Path, PathBuf},
};

type Configure = unsafe extern "C" fn(*const c_char, *const c_char) -> *mut c_char;
type Stop = unsafe extern "C" fn() -> *mut c_char;
type Free = unsafe extern "C" fn(*mut c_char);
type Version = unsafe extern "C" fn() -> *mut c_char;

pub fn library_path(dir: &Path) -> PathBuf {
    dir.join(if cfg!(windows) {
        "xrayc-core.dll"
    } else if cfg!(target_os = "macos") {
        "libxrayc-core.dylib"
    } else {
        "libxrayc-core.so"
    })
}

pub struct EmbeddedCore {
    _library: ManuallyDrop<Library>,
    validate: Configure,
    start: Configure,
    stop: Stop,
    free: Free,
    version: Version,
    assets: CString,
    active: bool,
}

impl EmbeddedCore {
    pub fn load(dir: &Path) -> io::Result<Self> {
        // Load only the application-owned library next to the service, never a
        // system search path or a path supplied by an imported configuration.
        let path = library_path(dir);
        let library = ManuallyDrop::new(unsafe { Library::new(&path) }.map_err(|error| {
            io::Error::other(format!(
                "Cannot load embedded Xray at {}: {error}",
                path.display()
            ))
        })?);
        unsafe {
            let abi = library
                .get::<unsafe extern "C" fn() -> u32>(b"XrayABIVersion\0")
                .map_err(io::Error::other)?;
            if abi() != 1 {
                return Err(io::Error::other(
                    "Incompatible embedded Xray ABI. Reinstall Xrayc.",
                ));
            }
            Ok(Self {
                validate: *library.get(b"XrayValidate\0").map_err(io::Error::other)?,
                start: *library.get(b"XrayStart\0").map_err(io::Error::other)?,
                stop: *library.get(b"XrayStop\0").map_err(io::Error::other)?,
                free: *library.get(b"XrayFree\0").map_err(io::Error::other)?,
                version: *library.get(b"XrayVersion\0").map_err(io::Error::other)?,
                assets: CString::new(dir.to_str().ok_or_else(|| {
                    io::Error::other("The application path must be valid Unicode")
                })?)?,
                _library: library,
                active: false,
            })
        }
    }

    fn take_string(&self, pointer: *mut c_char) -> Option<String> {
        if pointer.is_null() {
            return None;
        }
        // The ABI returns a NUL-terminated C allocation owned by the caller.
        let text = unsafe { CStr::from_ptr(pointer) }
            .to_string_lossy()
            .into_owned();
        unsafe { (self.free)(pointer) };
        Some(text)
    }

    fn checked(&self, pointer: *mut c_char) -> io::Result<()> {
        match self.take_string(pointer) {
            Some(error) => Err(io::Error::other(error)),
            None => Ok(()),
        }
    }

    fn configure(&self, operation: Configure, config: &Value) -> io::Result<()> {
        let config = CString::new(serde_json::to_vec(config)?)?;
        self.checked(unsafe { operation(config.as_ptr(), self.assets.as_ptr()) })
    }

    pub fn version(&self) -> String {
        self.take_string(unsafe { (self.version)() })
            .unwrap_or_default()
    }

    pub fn validate(&self, config: &Value) -> io::Result<()> {
        self.configure(self.validate, config)
    }

    pub fn start(&mut self, config: &Value) -> io::Result<()> {
        self.configure(self.start, config)?;
        self.active = true;
        Ok(())
    }

    pub fn stop(&mut self) -> io::Result<()> {
        if self.active {
            self.active = false;
            self.checked(unsafe { (self.stop)() })?;
        }
        Ok(())
    }
}

impl Drop for EmbeddedCore {
    fn drop(&mut self) {
        if let Err(error) = self.stop() {
            eprintln!("Failed to close embedded Xray: {error}");
        }
    }
}
