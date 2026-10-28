//! Loading modules: what `qi::import` does in `libqi`, for the libraries of Rust modules.
//!
//! Modules are found like `libqi` finds them (see [`Finder`]): a module `name` is declared by a
//! file `share/qi/module/<name>.mod` (containing `cpp`) in one of the SDK prefixes, and its
//! library is `lib/lib<name>.so` in one of them (a module named `a.b` is `lib/a/libb.so`).
//!
//! Libraries are never unloaded: their threads run until the process exits, as with `libqi`.

#![allow(unsafe_code)]

use super::abi::{self, ModuleV1};
use crate::{AnyObject, Error, Result};
use std::{
    ffi::{c_char, CStr, CString},
    mem::ManuallyDrop,
    path::{Path, PathBuf},
};
use tracing::debug;

/// What a module declaration says: its name, its type, and the path of its library.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleInfo {
    /// The name of the module.
    pub name: String,
    /// The type of the module, as written in its `.mod` file: `cpp` for the libraries `libqi`
    /// loads (Rust modules included).
    pub kind: String,
    /// The library of the module.
    pub path: PathBuf,
}

/// Finds modules in SDK prefixes, the directories `libqi` searches for its data and libraries,
/// with the algorithm of `libqi` (`qi::path::findData`, `qi::path::findLib`).
#[derive(Debug, Clone, Default)]
pub struct Finder {
    prefixes: Vec<PathBuf>,
    writable_data: Option<PathBuf>,
}

impl Finder {
    /// A finder searching the given prefixes, in order, and no writable data directory.
    pub fn new(prefixes: impl IntoIterator<Item = PathBuf>) -> Self {
        Self {
            prefixes: prefixes.into_iter().collect(),
            writable_data: None,
        }
    }

    /// The prefixes of `libqi`, in order:
    ///
    /// 1. the SDK prefix: the `--qi-sdk-prefix` option if given, else the `QI_SDK_PREFIX`
    ///    environment variable;
    /// 2. the prefix of the program: the parent of the directory of the executable;
    /// 3. the prefixes of `QI_ADDITIONAL_SDK_PREFIXES` (separated by `:`);
    /// 4. the directories listed in `share/qi/path.conf` of the first prefix.
    ///
    /// Module declarations are first searched in the writable data directory of `libqi`:
    /// `$QI_WRITABLE_PATH/data`, else `~/.local/share`.
    pub fn from_environment(sdk_prefix: Option<PathBuf>) -> Self {
        let mut prefixes = Vec::new();
        prefixes
            .extend(sdk_prefix.or_else(|| std::env::var_os("QI_SDK_PREFIX").map(PathBuf::from)));
        if let Some(prefix) = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent()?.parent().map(Path::to_owned))
        {
            prefixes.push(prefix);
        }
        if let Some(additional) = std::env::var_os("QI_ADDITIONAL_SDK_PREFIXES") {
            prefixes
                .extend(std::env::split_paths(&additional).filter(|p| !p.as_os_str().is_empty()));
        }
        if let Some(first) = prefixes.first().cloned() {
            read_path_conf(&first.join("share/qi/path.conf"), &mut prefixes);
        }
        let mut unique = Vec::new();
        for prefix in prefixes {
            if !unique.contains(&prefix) {
                unique.push(prefix);
            }
        }
        let writable_data = std::env::var_os("QI_WRITABLE_PATH")
            .map(|path| PathBuf::from(path).join("data"))
            .or_else(|| {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share"))
            });
        Self {
            prefixes: unique,
            writable_data,
        }
    }

    /// The prefixes searched, in order.
    pub fn prefixes(&self) -> &[PathBuf] {
        &self.prefixes
    }

    /// Finds a module by name, failing like `libqi` (`no module found: 'name'`) when no
    /// declaration `qi/module/<name>.mod` is found (the dots of the name are kept).
    pub fn find(&self, name: &str) -> Result<ModuleInfo> {
        check_module_name(name)?;
        let file = format!("{name}.mod");
        let declaration = self
            .writable_data
            .iter()
            .map(|dir| dir.join("qi/module").join(&file))
            .chain(
                self.prefixes
                    .iter()
                    .map(|prefix| prefix.join("share/qi/module").join(&file)),
            )
            .find(|path| path.is_file())
            .ok_or_else(|| Error::from(format!("no module found: '{name}'")))?;
        let kind = std::fs::read_to_string(&declaration)?
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .to_owned();
        debug!(module = name, declaration = %declaration.display(), %kind, "module found");
        if kind != "cpp" {
            return Err(format!("module factory for module type: {kind} is not available").into());
        }
        // A module `a.b` is the library `b` of the directory `a`.
        let relative = name.replace('.', "/");
        let path = self
            .find_library(&relative)
            .unwrap_or_else(|| PathBuf::from(&relative));
        Ok(ModuleInfo {
            name: name.to_owned(),
            kind,
            path,
        })
    }

    /// Finds a library like `qi::path::findLib`: `<relative>` from the current directory, then,
    /// in the `lib` directory of each prefix, `<dir>/<name>`, `<dir>/<name>.so`,
    /// `<dir>/lib<name>.so` and `<dir>/lib<name>`.
    pub fn find_library(&self, relative: &str) -> Option<PathBuf> {
        let relative_path = Path::new(relative);
        if relative_path.is_file() {
            return Some(relative_path.to_owned());
        }
        let dir = relative_path.parent().unwrap_or(Path::new(""));
        let file = relative_path.file_name()?.to_string_lossy().into_owned();
        let candidates = [
            file.clone(),
            format!("{file}.so"),
            format!("lib{file}.so"),
            format!("lib{file}"),
        ];
        self.prefixes.iter().find_map(|prefix| {
            candidates
                .iter()
                .map(|candidate| prefix.join("lib").join(dir).join(candidate))
                .find(|path| path.is_file())
        })
    }
}

/// Adds the existing directories listed in a `path.conf` file (one per line, `#` comments),
/// and those of the `path.conf` files of these directories.
fn read_path_conf(path: &Path, prefixes: &mut Vec<PathBuf>) {
    let Ok(content) = std::fs::read_to_string(path) else {
        return;
    };
    for line in content.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let prefix = PathBuf::from(line);
        if prefix.is_dir() && !prefixes.contains(&prefix) {
            prefixes.push(prefix.clone());
            read_path_conf(&prefix.join("share/qi/path.conf"), prefixes);
        }
    }
}

/// Module names are made of `[_.a-zA-Z0-9]`, as `libqi` requires.
fn check_module_name(name: &str) -> Result<()> {
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_')
    {
        return Err(format!(
            "Invalid module name: '{name}', use only character from [_.a-zA-Z0-9]"
        )
        .into());
    }
    Ok(())
}

/// A loaded module.
pub struct Module {
    name: String,
    factories: Vec<String>,
    path: PathBuf,
    object: AnyObject,
    _node: crate::Node<crate::service_directory::Client>,
    _library: ManuallyDrop<libloading::Library>,
}

impl Module {
    /// Loads the library of a Rust module and connects to it. The space URL, if given, is the
    /// one the session of the module connects to (the space of the host).
    pub async fn load(path: impl Into<PathBuf>, space_url: Option<&str>) -> Result<Self> {
        let path = path.into();
        let space_url = space_url.map(str::to_owned);
        let (library, name, factories, endpoint) = tokio::task::spawn_blocking({
            let path = path.clone();
            move || open(&path, space_url.as_deref())
        })
        .await
        .map_err(|err| Error::Other(err.into()))??;
        let address = endpoint
            .parse()
            .map_err(|err| Error::from(format!("invalid module endpoint {endpoint:?}: {err}")))?;
        let node = crate::node::init()
            .connect_to_space(address, None)
            .start()
            .await?;
        let object = node.service(&name).await?;
        debug!(module = %name, path = %path.display(), %endpoint, "module loaded");
        Ok(Self {
            name,
            factories,
            path,
            object,
            _node: node,
            _library: ManuallyDrop::new(library),
        })
    }

    /// The name of the module.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The names of the factories of the module, in declaration order.
    pub fn factories(&self) -> &[String] {
        &self.factories
    }

    /// The path of the library of the module.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The module object: its methods are the functions and the factories of the module.
    pub fn object(&self) -> &AnyObject {
        &self.object
    }
}

impl std::fmt::Debug for Module {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Module")
            .field("name", &self.name)
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

/// Opens a library, reads its descriptor and starts the module. Blocking.
fn open(
    path: &Path,
    space_url: Option<&str>,
) -> Result<(libloading::Library, String, Vec<String>, String)> {
    let failure =
        |message: String| Error::from(format!("Could not open {}: {message}", path.display()));
    // `libqi` opens modules with `RTLD_NOW`.
    // SAFETY: the initializers of a library run when it is loaded; loading a module is running
    // its code, which is what the caller asks for.
    let library: libloading::Library = unsafe {
        libloading::os::unix::Library::open(
            Some(path),
            libloading::os::unix::RTLD_NOW | libloading::os::unix::RTLD_LOCAL,
        )
    }
    .map_err(|err| failure(err.to_string()))?
    .into();
    // SAFETY: the symbol, when the library exports it, has this signature (`#[qi::module]`).
    let entry = unsafe {
        library.get::<extern "C" fn() -> *const ModuleV1>(abi::ENTRY_POINT_V1.as_bytes())
    }
    .map_err(|err| {
        failure(format!(
            "not a Rust module ({err}); C++ modules are loaded by libqi hosts, such as qilaunch"
        ))
    })?;
    // SAFETY: the descriptor is a static of the library, which stays loaded.
    let descriptor = unsafe { &*entry() };
    if descriptor.abi_version != abi::ABI_VERSION {
        return Err(failure(format!(
            "unsupported module ABI version {} (expected {})",
            descriptor.abi_version,
            abi::ABI_VERSION
        )));
    }
    // SAFETY: the name of the descriptor is a static NUL-terminated string.
    let name = unsafe { CStr::from_ptr(descriptor.name) }
        .to_string_lossy()
        .into_owned();
    let factories = (0..descriptor.factory_count)
        .map(|index| {
            // SAFETY: the factories of the descriptor are a static array of `factory_count`
            // elements with static NUL-terminated names.
            unsafe { CStr::from_ptr((*descriptor.factories.add(index)).name) }
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    if let Some(url) = space_url {
        let key = CString::new("space-url").expect("no NUL");
        let value = CString::new(url).map_err(|err| failure(err.to_string()))?;
        // SAFETY: the contract of `ModuleV1::set_option`: NUL-terminated strings.
        if unsafe { (descriptor.set_option)(key.as_ptr(), value.as_ptr()) } != 0 {
            return Err(failure(format!("the module rejected the space URL {url}")));
        }
    }
    let mut out = vec![0u8; 4096];
    // SAFETY: the contract of `ModuleV1::start`: a buffer of the given length.
    let status = unsafe { (descriptor.start)(out.as_mut_ptr().cast::<c_char>(), out.len()) };
    let text = CStr::from_bytes_until_nul(&out)
        .map(|text| text.to_string_lossy().into_owned())
        .unwrap_or_default();
    if status != 0 {
        return Err(failure(format!("the module failed to start: {text}")));
    }
    Ok((library, name, factories, text))
}
