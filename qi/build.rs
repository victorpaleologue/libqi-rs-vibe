//! Builds the C++ bridges of modules against the SDKs of their hosts.
//!
//! - Feature `module-cpp`: the libqi entry points (`cpp/module_shim.cpp`), against the libqi SDK
//!   named by `QI_MODULE_CPP_SDK` (an installation prefix, with `include` and `lib` directories).
//!   The API follows the SDK: libqi 2.3 and later (`qi/anymodule.hpp`) or libqi 2.1
//!   (`qitype/objectfactory.hpp`).
//! - Feature `module-naoqi`: the `ALModule` of DCM plugins (`cpp/naoqi_shim.cpp`), against the
//!   C++ SDK of NAOqi named by `QI_MODULE_NAOQI_SDK` (the `naoqi-sdk`, or the sysroot of the cross
//!   toolchain of the robot), linked with `alcommon`, `alproxies`, `alvalue` and `alerror` unless
//!   `QI_MODULE_NAOQI_LIBS` says otherwise.
//!
//! For each of them, `<PREFIX>_INCLUDES` adds include directories (separated by `:`),
//! `<PREFIX>_FLAGS` compiler flags (separated by spaces, such as `-D_GLIBCXX_USE_CXX11_ABI=0`
//! or `--sysroot=...`) and `<PREFIX>_LIBS` libraries (separated by `:`), where `<PREFIX>` is
//! `QI_MODULE_CPP` or `QI_MODULE_NAOQI`. The compiler is the one of the `cc` crate (`CXX`,
//! `CXXFLAGS` and their target-specific variants).

fn main() {
    #[cfg(feature = "module")]
    glibc_compat();
    #[cfg(feature = "module-cpp")]
    bridge::build_libqi();
    #[cfg(feature = "module-naoqi")]
    bridge::build_naoqi();
}

/// Links the fallback of `getauxval` (`cpp/glibc_compat.c`) into the programs and libraries
/// using modules, on Linux with the GNU C library, for the libraries of modules to load with the
/// glibc older than 2.16 of NAOqi 2.1 robots.
#[cfg(feature = "module")]
fn glibc_compat() {
    println!("cargo:rerun-if-changed=cpp/glibc_compat.c");
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if os != "linux" || env != "gnu" {
        return;
    }
    cc::Build::new()
        .file("cpp/glibc_compat.c")
        .pic(true)
        .cargo_metadata(false)
        .compile("qi_glibc_compat");
    // The whole archive: its definition must be linked before the standard library, which
    // references it, is.
    let out = std::env::var("OUT_DIR").expect("cargo sets OUT_DIR");
    println!("cargo:rustc-link-search=native={out}");
    println!("cargo:rustc-link-lib=static:+whole-archive=qi_glibc_compat");
}

#[cfg(any(feature = "module-cpp", feature = "module-naoqi"))]
mod bridge {
    use std::{
        env,
        path::{Path, PathBuf},
    };

    struct Settings {
        sdk: PathBuf,
        build: cc::Build,
        libs: Option<String>,
    }

    fn settings(prefix: &str, source: &str, what: &str) -> Settings {
        for suffix in ["SDK", "INCLUDES", "FLAGS", "LIBS"] {
            println!("cargo:rerun-if-env-changed={prefix}_{suffix}");
        }
        println!("cargo:rerun-if-changed={source}");
        let sdk = PathBuf::from(env::var_os(format!("{prefix}_SDK")).unwrap_or_else(|| {
            panic!(
                "{what}: set {prefix}_SDK to the installation prefix of the SDK (the directory \
                 of its `include` and `lib` directories)"
            )
        }));
        let mut build = cc::Build::new();
        build
            .cpp(true)
            .file(source)
            .include(sdk.join("include"))
            .warnings(false)
            .pic(true)
            // Boost 1.73 and later warn about the global placeholders of `boost::bind`, which the
            // Boost of old SDKs only has.
            .define("BOOST_BIND_GLOBAL_PLACEHOLDERS", None);
        if let Some(includes) = env::var_os(format!("{prefix}_INCLUDES")) {
            for dir in env::split_paths(&includes) {
                build.include(dir);
            }
        }
        if let Ok(flags) = env::var(format!("{prefix}_FLAGS")) {
            for flag in flags.split_whitespace() {
                build.flag(flag);
            }
        }
        let libs = env::var(format!("{prefix}_LIBS")).ok();
        Settings { sdk, build, libs }
    }

    fn link(sdk: &Path, defaults: &[&str], libs: Option<String>) {
        println!(
            "cargo:rustc-link-search=native={}",
            sdk.join("lib").display()
        );
        let libs = match libs {
            Some(libs) => libs
                .split(':')
                .filter(|lib| !lib.is_empty())
                .map(str::to_owned)
                .collect(),
            None => defaults
                .iter()
                .map(|lib| (*lib).to_owned())
                .collect::<Vec<_>>(),
        };
        for lib in libs {
            println!("cargo:rustc-link-lib=dylib={lib}");
        }
    }

    #[cfg(feature = "module-cpp")]
    pub(super) fn build_libqi() {
        let Settings {
            sdk,
            mut build,
            libs,
        } = settings(
            "QI_MODULE_CPP",
            "cpp/module_shim.cpp",
            "the feature `module-cpp` builds the libqi entry points of modules against a libqi SDK",
        );
        let include = sdk.join("include");
        let libqi21 = include.join("qitype/objectfactory.hpp").is_file();
        if !libqi21 && !include.join("qi/anymodule.hpp").is_file() {
            panic!(
                "{} is not a libqi SDK: neither qi/anymodule.hpp (libqi 2.3 and later) nor \
                 qitype/objectfactory.hpp (libqi 2.1) is in its include directory",
                sdk.display()
            );
        }
        if libqi21 {
            build.define("QI_RS_SHIM_LIBQI21", None);
        } else if include.join("qi/messaging/tcpscheme.hpp").is_file() {
            // libqi 3 and 4 require C++17, with the GNU extensions their log macros use.
            build.flag_if_supported("-std=gnu++17");
        } else if build.is_flag_supported("-std=gnu++14").unwrap_or(false) {
            // libqi 2.3 to 2.9, whose headers are not C++17 (libqi 2.9 with clang).
            build.flag("-std=gnu++14");
        } else {
            build.flag_if_supported("-std=gnu++11");
        }
        build.compile("qi_module_shim");
        let defaults: &[&str] = if libqi21 {
            &["qimessaging", "qitype", "qi"]
        } else {
            &["qi"]
        };
        link(&sdk, defaults, libs);
    }

    #[cfg(feature = "module-naoqi")]
    pub(super) fn build_naoqi() {
        let Settings { sdk, build, libs } = settings(
            "QI_MODULE_NAOQI",
            "cpp/naoqi_shim.cpp",
            "the feature `module-naoqi` builds the DCM plugins of modules against the C++ SDK of \
             NAOqi",
        );
        if !sdk.join("include/alcommon/almodule.h").is_file() {
            panic!(
                "{} is not a NAOqi C++ SDK: alcommon/almodule.h is not in its include directory",
                sdk.display()
            );
        }
        build.compile("qi_naoqi_shim");
        link(&sdk, &["alcommon", "alproxies", "alvalue", "alerror"], libs);
    }
}
