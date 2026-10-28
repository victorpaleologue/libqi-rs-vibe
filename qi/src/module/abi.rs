//! The entry points of the libraries of Rust modules.
//!
//! [`#[qi::module]`](macro@crate::module) makes the library export these C symbols:
//!
//! | Symbol | Caller | Effect |
//! |---|---|---|
//! | `qi_rs_module_v1` | hosts written with this crate (`qi-launch`) | returns the [`ModuleV1`] descriptor of the module |
//! | `_createModule`, `_closeModule` | NAOqi, for the libraries of its `autoload.ini` (the entry points of the `ALModule`s of `libalcommon`) | registers the objects of the `autoload` factories as services of the local NAOqi; unregisters them |
//! | `qi_module_init` (feature `module-cpp`) | `libqi` 2.3 and later (`qi::import`, `qilaunch`, `Session::loadService`, Python's `qi.module`) | fills the `qi::ModuleBuilder` of the module with forwarders to its members |
//!
//! The descriptor and the entry points only exchange C strings and integers: the members of the
//! module are reached through the private space whose endpoint [`ModuleV1::start`] returns,
//! with the messaging protocol. The ABI is versioned by the symbol name and by
//! [`ModuleV1::abi_version`].
//!
//! `_createModule` is declared by NAOqi as `extern "C" int _createModule(boost::shared_ptr<AL::ALBroker>)`.
//! The C++ ABI of Linux (Itanium) passes a parameter of a class type that is not trivially
//! copyable as a pointer to a temporary the caller owns, so the Rust implementation takes a
//! pointer, which it ignores: the module reaches NAOqi through its network endpoint, not through
//! the broker.

#![allow(unsafe_code)]

use super::ModuleDef;
use std::ffi::{c_char, c_int, c_void, CStr};

/// The name of the symbol returning the [`ModuleV1`] descriptor of a module.
pub const ENTRY_POINT_V1: &str = "qi_rs_module_v1";

/// The version of the ABI the descriptor of [`ENTRY_POINT_V1`] follows.
pub const ABI_VERSION: u32 = 1;

/// The descriptor of a module, returned by the `qi_rs_module_v1` symbol of its library.
#[repr(C)]
pub struct ModuleV1 {
    /// [`ABI_VERSION`].
    pub abi_version: u32,
    /// The name of the module, NUL-terminated.
    pub name: *const c_char,
    /// Sets an option of the module, before it starts; both strings are NUL-terminated.
    /// Returns 0 when the option is set, -1 when it is unknown or invalid. The options are:
    ///
    /// - `space-url`: the URL of the space of the host, that the [`Session`](super::Session)
    ///   of the module connects to, unless already connected;
    /// - `listen-url`: the URL the session listens on, for the services the module registers;
    /// - `machine-id`: the identifier of the local machine, as the host computes it, which the
    ///   module advertises in its service infos (`libqi` uses only the loopback endpoints of
    ///   services of its own machine).
    pub set_option: unsafe extern "C" fn(key: *const c_char, value: *const c_char) -> c_int,
    /// Starts the module (idempotent) and writes the URL of the endpoint of its private space,
    /// NUL-terminated, in `out` (of capacity `out_len`). Returns 0 on success; on failure,
    /// returns -1 and writes the error message in `out`.
    pub start: unsafe extern "C" fn(out: *mut c_char, out_len: usize) -> c_int,
    /// The number of factories of the module.
    pub factory_count: usize,
    /// The factories of the module: an array of `factory_count` elements.
    pub factories: *const FactoryV1,
}

/// A factory of a module, in its [`ModuleV1`] descriptor.
#[repr(C)]
pub struct FactoryV1 {
    /// The name of the factory, NUL-terminated.
    pub name: *const c_char,
    /// [`FACTORY_AUTOLOAD`] when the object of the factory is registered by `_createModule`.
    pub flags: u32,
}

/// The flag of the factories whose object is registered as a service when NAOqi loads the
/// library.
pub const FACTORY_AUTOLOAD: u32 = 1;

// The descriptors are immutable statics pointing to static strings.
unsafe impl Sync for ModuleV1 {}
unsafe impl Sync for FactoryV1 {}

/// The implementation of [`ModuleV1::set_option`].
///
/// # Safety
/// `key` and `value` are null or NUL-terminated strings.
#[doc(hidden)]
pub unsafe fn set_option(
    module: &'static ModuleDef,
    key: *const c_char,
    value: *const c_char,
) -> c_int {
    if key.is_null() || value.is_null() {
        return -1;
    }
    // SAFETY: non-null `key` and `value` are NUL-terminated strings.
    let (key, value) = unsafe { (CStr::from_ptr(key), CStr::from_ptr(value)) };
    let (key, value) = (key.to_string_lossy(), value.to_string_lossy());
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        module.set_option(&key, &value)
    })) {
        Ok(true) => 0,
        _ => -1,
    }
}

/// The implementation of [`ModuleV1::start`].
///
/// # Safety
/// `out` points to `out_len` writable bytes.
#[doc(hidden)]
pub unsafe fn start(module: &'static ModuleDef, out: *mut c_char, out_len: usize) -> c_int {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| module.start()))
        .unwrap_or_else(|_| Err("the module panicked while starting".into()));
    let (status, text) = match result {
        Ok(endpoint) => (0, endpoint),
        Err(err) => (-1, err.to_string()),
    };
    // SAFETY: `out` points to `out_len` writable bytes.
    unsafe { write_c_string(&text, out, out_len) };
    status
}

/// The implementation of `_createModule`.
#[doc(hidden)]
pub fn create_module(module: &'static ModuleDef, _broker: *mut c_void) -> c_int {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| module.autoload())) {
        Ok(()) => 0,
        Err(_) => -1,
    }
}

/// The implementation of `_closeModule`.
#[doc(hidden)]
pub fn close_module(module: &'static ModuleDef) -> c_int {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| module.close())) {
        Ok(()) => 0,
        Err(_) => -1,
    }
}

/// Writes a string, truncated if needed, NUL-terminated, to a buffer.
///
/// # Safety
/// `out` is null or points to `out_len` writable bytes.
unsafe fn write_c_string(text: &str, out: *mut c_char, out_len: usize) {
    if out.is_null() || out_len == 0 {
        return;
    }
    let bytes = text.as_bytes();
    let len = bytes.len().min(out_len - 1);
    // SAFETY: `len + 1 <= out_len` bytes are written to `out`.
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr().cast::<c_char>(), out, len);
        *out.add(len) = 0;
    }
}

/// Exports the entry points of a module from the library. Used by `#[qi::module]`.
#[doc(hidden)]
#[macro_export]
macro_rules! __export_module {
    (
        def: $def:path,
        name: $name:expr,
        factories: [$(($factory:expr, $flags:expr)),* $(,)?] $(,)?
    ) => {
        #[allow(unsafe_code)]
        const _: () = {
            static __QI_MODULE_FACTORIES: [$crate::module::abi::FactoryV1; <[u32]>::len(&[$($flags),*])] = [
                $($crate::module::abi::FactoryV1 {
                    name: ::std::concat!($factory, "\0").as_ptr().cast(),
                    flags: $flags,
                }),*
            ];

            static __QI_MODULE_V1: $crate::module::abi::ModuleV1 = $crate::module::abi::ModuleV1 {
                abi_version: $crate::module::abi::ABI_VERSION,
                name: ::std::concat!($name, "\0").as_ptr().cast(),
                set_option: __qi_module_set_option,
                start: __qi_module_start,
                factory_count: __QI_MODULE_FACTORIES.len(),
                factories: __QI_MODULE_FACTORIES.as_ptr(),
            };

            unsafe extern "C" fn __qi_module_set_option(
                key: *const ::std::ffi::c_char,
                value: *const ::std::ffi::c_char,
            ) -> ::std::ffi::c_int {
                // SAFETY: the contract of `ModuleV1::set_option`.
                unsafe { $crate::module::abi::set_option(&$def, key, value) }
            }

            unsafe extern "C" fn __qi_module_start(
                out: *mut ::std::ffi::c_char,
                out_len: usize,
            ) -> ::std::ffi::c_int {
                // SAFETY: the contract of `ModuleV1::start`.
                unsafe { $crate::module::abi::start(&$def, out, out_len) }
            }

            /// The descriptor of the module, for hosts written with the `qi` crate.
            #[unsafe(no_mangle)]
            pub extern "C" fn qi_rs_module_v1() -> *const $crate::module::abi::ModuleV1 {
                &__QI_MODULE_V1
            }

            /// The entry point of NAOqi for the libraries of its `autoload.ini`.
            #[unsafe(no_mangle)]
            pub extern "C" fn _createModule(broker: *mut ::std::ffi::c_void) -> ::std::ffi::c_int {
                $crate::module::abi::create_module(&$def, broker)
            }

            /// The exit point of NAOqi for the libraries of its `autoload.ini`.
            #[unsafe(no_mangle)]
            pub extern "C" fn _closeModule() -> ::std::ffi::c_int {
                $crate::module::abi::close_module(&$def)
            }

            $crate::__export_module_cpp!(__QI_MODULE_V1);
        };
    };
}

/// Exports the entry points of `libqi` (feature `module-cpp`).
#[cfg(feature = "module-cpp")]
#[doc(hidden)]
#[macro_export]
macro_rules! __export_module_cpp {
    ($descriptor:ident) => {
        /// The entry point of `libqi` 2.3 and later (`QI_REGISTER_MODULE`).
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn qi_module_init(builder: *mut ::std::ffi::c_void) {
            // SAFETY: `libqi` gives its `qi::ModuleBuilder`.
            unsafe { $crate::module::abi::cpp::module_init(&$descriptor, builder) }
        }

        // The registration of the factories in `libqi` 2.1 (`QI_REGISTER_OBJECT_FACTORY`), at
        // the loading of the library; nothing with later versions.
        #[used]
        #[unsafe(link_section = ".init_array")]
        static __QI_MODULE_CPP_STATIC_INIT: extern "C" fn() = {
            extern "C" fn __qi_module_cpp_static_init() {
                $crate::module::abi::cpp::static_init(&$descriptor)
            }
            __qi_module_cpp_static_init
        };
    };
}

/// Exports the entry points of `libqi` (feature `module-cpp`, disabled).
#[cfg(not(feature = "module-cpp"))]
#[doc(hidden)]
#[macro_export]
macro_rules! __export_module_cpp {
    ($descriptor:ident) => {};
}

#[cfg(feature = "module-cpp")]
#[doc(hidden)]
pub mod cpp;
