//! The calls of the entry points of `libqi` into the C++ shim (feature `module-cpp`), built
//! against the libqi SDK named by `QI_MODULE_CPP_SDK` (see `cpp/module_shim.cpp`).

use super::ModuleV1;
use std::ffi::c_void;

unsafe extern "C" {
    fn qi_rs_shim_module_init(module: *const ModuleV1, builder: *mut c_void);
    fn qi_rs_shim_static_init(module: *const ModuleV1);
}

/// The implementation of `qi_module_init` (libqi 2.3 and later).
///
/// # Safety
/// `builder` is the `qi::ModuleBuilder` `libqi` gives.
#[doc(hidden)]
pub unsafe fn module_init(module: &'static ModuleV1, builder: *mut c_void) {
    // SAFETY: the shim catches every C++ exception; the descriptor is a static.
    unsafe { qi_rs_shim_module_init(module, builder) }
}

/// The registration of the factories when the library is loaded (libqi 2.1).
#[doc(hidden)]
pub fn static_init(module: &'static ModuleV1) {
    // SAFETY: the shim catches every C++ exception; the descriptor is a static.
    unsafe { qi_rs_shim_static_init(module) }
}
