//! Builds the C++ shim of modules (feature `module-cpp`) against a libqi SDK.
//!
//! The SDK is the installation prefix named by `QI_MODULE_CPP_SDK` (its `include` and `lib`
//! directories). The API of the shim follows the SDK: libqi 2.3 and later (`qi/anymodule.hpp`)
//! or libqi 2.1 (`qitype/objectfactory.hpp`). Other variables:
//!
//! - `QI_MODULE_CPP_INCLUDES`: more include directories (Boost...), separated by `:`;
//! - `QI_MODULE_CPP_FLAGS`: more compiler flags, separated by spaces (such as
//!   `-D_GLIBCXX_USE_CXX11_ABI=0` or `--sysroot=...`);
//! - `QI_MODULE_CPP_LIBS`: more libraries to link, separated by `:` (`boost_system`...);
//! - `CXX`, `CXXFLAGS` (and their target-specific variants): the compiler, as for the `cc` crate.

fn main() {
    #[cfg(feature = "module-cpp")]
    shim::build();
}

#[cfg(feature = "module-cpp")]
mod shim {
    use std::{env, path::PathBuf};

    pub(super) fn build() {
        for var in [
            "QI_MODULE_CPP_SDK",
            "QI_MODULE_CPP_INCLUDES",
            "QI_MODULE_CPP_FLAGS",
            "QI_MODULE_CPP_LIBS",
        ] {
            println!("cargo:rerun-if-env-changed={var}");
        }
        println!("cargo:rerun-if-changed=cpp/module_shim.cpp");

        let sdk = PathBuf::from(env::var_os("QI_MODULE_CPP_SDK").unwrap_or_else(|| {
            panic!(
                "the feature `module-cpp` builds the libqi entry points of modules against a \
                 libqi SDK: set QI_MODULE_CPP_SDK to its installation prefix (the directory of \
                 its `include` and `lib` directories)"
            )
        }));
        let include = sdk.join("include");
        let libqi21 = include.join("qitype/objectfactory.hpp").is_file();
        if !libqi21 && !include.join("qi/anymodule.hpp").is_file() {
            panic!(
                "{} is not a libqi SDK: neither qi/anymodule.hpp (libqi 2.3 and later) nor \
                 qitype/objectfactory.hpp (libqi 2.1) is in its include directory",
                sdk.display()
            );
        }

        let mut build = cc::Build::new();
        build
            .cpp(true)
            .file("cpp/module_shim.cpp")
            .include(&include)
            .warnings(false)
            .pic(true);
        if let Some(includes) = env::var_os("QI_MODULE_CPP_INCLUDES") {
            for dir in env::split_paths(&includes) {
                build.include(dir);
            }
        }
        if libqi21 {
            build.define("QI_RS_SHIM_LIBQI21", None);
        } else {
            // libqi 4 requires C++17, with the GNU extensions its log macros use.
            build.flag_if_supported("-std=gnu++17");
        }
        if let Ok(flags) = env::var("QI_MODULE_CPP_FLAGS") {
            for flag in flags.split_whitespace() {
                build.flag(flag);
            }
        }
        build.compile("qi_module_shim");

        println!(
            "cargo:rustc-link-search=native={}",
            sdk.join("lib").display()
        );
        let libraries: &[&str] = if libqi21 {
            &["qimessaging", "qitype", "qi"]
        } else {
            &["qi"]
        };
        for library in libraries {
            println!("cargo:rustc-link-lib=dylib={library}");
        }
        if let Ok(libs) = env::var("QI_MODULE_CPP_LIBS") {
            for library in libs.split(':').filter(|library| !library.is_empty()) {
                println!("cargo:rustc-link-lib=dylib={library}");
            }
        }
    }
}
