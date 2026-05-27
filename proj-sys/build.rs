use flate2::read::GzDecoder;
use std::env;
use std::fs::{self, File};
use std::path::PathBuf;
use tar::Archive;

const MINIMUM_PROJ_VERSION: &str = "9.6.0";

#[cfg(feature = "nobuild")]
fn main() {} // Skip the build script on docs.rs

#[cfg(not(feature = "nobuild"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let include_path = if cfg!(feature = "bundled_proj") {
        eprintln!("feature flags specified source build");
        build_from_source()?
    } else {
        pkg_config::Config::new()
        .atleast_version(MINIMUM_PROJ_VERSION)
        .probe("proj")
        .map(|pk| {
            eprintln!("found acceptable libproj already installed at: {:?}", pk.link_paths[0]);
            if cfg!(feature = "network") {
                // Generally, system proj installations have been built with tiff support
                // allowing for network grid interaction. If this proves to be untrue
                // could we try to determine some kind of runtime check and fall back
                // to building from source?
                eprintln!("assuming existing system libproj installation has network (tiff) support");
            }
            if let Ok(val) = &env::var("_PROJ_SYS_TEST_EXPECT_BUILD_FROM_SRC") {
                if val != "0" {
                    panic!("for testing purposes: existing package was found, but should not have been");
                }
            }

            // Tell cargo to tell rustc to link the system proj
            // shared library.
            println!("cargo:rustc-link-search=native={:?}", pk.link_paths[0]);
            println!("cargo:rustc-link-lib=proj");

            pk.include_paths[0].clone()
        })
        .or_else(|err| {
            eprintln!("pkg-config unable to find existing libproj installation: {err}");
            build_from_source()
        })?
    };

    generate_bindings(include_path)?;
    Ok(())
}

fn generate_bindings(include_path: std::path::PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    // The bindgen::Builder is the main entry point
    // to bindgen, and lets you build up options for
    // the resulting bindings.
    // If you update the configuration here you also
    // need to update the corresponding bindgen command in
    // `DEVELOPMENT.md`

    let bindings = bindgen::Builder::default()
        .clang_arg(format!("-I{}", include_path.to_string_lossy()))
        .size_t_is_usize(true)
        .blocklist_type("max_align_t");

    let bindings = {
        let ndk_path = env::var("ANDROID_NDK").expect("ANDROID_NDK not set");
        let target = env::var("TARGET").expect("TARGET not set");
        let host = env::var("HOST").expect("OS not set");
        let host_parts: Vec<&str> = host.split("-").collect();

        if target.contains("android") {
            let llvm_bindir = format!("{}/toolchains/llvm/prebuilt/{}-{}/bin", ndk_path, std::env::consts::OS, host_parts[0]);

            eprintln!("LIBCLANG_PATH={}", llvm_bindir);
            eprintln!("sysroot={}", llvm_bindir.replace("/bin", ""));

            println!("cargo:rustc-env=LIBCLANG_PATH={}", llvm_bindir);
            println!("cargo:rustc-env=CLANG_PATH={}", llvm_bindir);
            println!("cargo:rerun-if-changed=wrapper.h");

            bindings
                .header("wrapper.h")
                .clang_arg(format!("--target={}", target))
                .clang_arg(format!("--sysroot={}/sysroot", llvm_bindir.replace("/bin", "")))
                .clang_arg(format!("-I{}/sysroot/usr/include", llvm_bindir.replace("/bin", "")))
                .clang_arg(format!("-I{}/lib/clang/21/include", llvm_bindir.replace("/bin", "")))
        } else {
            bindings
        }
    };

    let bindings = bindings
        // The input header we would like to generate
        // bindings for.
        .header("wrapper.h")
        // Finish the builder and generate the bindings.
        .generate()
        // Unwrap the Result and panic on failure.
        .expect("Unable to generate bindings");

    // Write the bindings to the $OUT_DIR/bindings.rs file.
    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
    bindings.write_to_file(out_path.join("bindings.rs"))?;

    Ok(())
}

// returns the path of "include" for the built proj
fn build_from_source() -> Result<std::path::PathBuf, Box<dyn std::error::Error>> {
    eprintln!("building libproj from source");
    println!("cargo:rustc-cfg=bundled_build");
    println!("cargo:rustc-link-arg=-static-libstdc++");
    if let Ok(val) = &env::var("_PROJ_SYS_TEST_EXPECT_BUILD_FROM_SRC") {
        if val == "0" {
            panic!(
                "for testing purposes: package was building from source but should not have been"
            );
        }
    }

    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());

    let zlib_paths = std::env::var("DEP_Z_ROOT").ok().map(|zlib_root_dir| {
        let zlib_root = PathBuf::from(zlib_root_dir);
        let zlib_include = zlib_root.join("include");
        let zlib_lib_dir = zlib_root.join("lib");
        let zlib_library = if env::var("TARGET")
            .unwrap_or_default()
            .contains("windows-msvc")
        {
            ["zlibstaticd.lib", "zlibstatic.lib", "z.lib"]
                .into_iter()
                .map(|name| zlib_lib_dir.join(name))
                .find(|path| path.exists())
                .unwrap_or_else(|| zlib_lib_dir.join("zlibstatic.lib"))
        } else {
            zlib_lib_dir.join("libz.a")
        };

        let zlib_link_name = zlib_library
            .file_stem()
            .and_then(|name| name.to_str())
            .map(|name| name.strip_prefix("lib").unwrap_or(name).to_string())
            .unwrap_or_else(|| "z".to_string());

        (
            zlib_root,
            zlib_include,
            zlib_lib_dir,
            zlib_library,
            zlib_link_name,
        )
    });
    if cfg!(feature = "tiff") && zlib_paths.is_none() {
        panic!("feature 'tiff' requires bundled static zlib from libz-sys, but DEP_Z_ROOT was not provided");
    }

    let (tiff_include, tiff_lib_dir) = if cfg!(feature = "tiff") {
        eprintln!("feature 'tiff' enabled — building libtiff from source");

        let tiff_src = PathBuf::from("PROJSRC/libtiff");
        if !tiff_src.exists() {
            panic!(
                "Missing libtiff source directory at {:?}. Did you vendor or clone it?",
                tiff_src
            );
        }

        let mut tiff_cfg = cmake::Config::new(&tiff_src);
        tiff_cfg.profile("Release");
        tiff_cfg.define("BUILD_SHARED_LIBS", "OFF");
        tiff_cfg.define("tiff-tools", "OFF");
        tiff_cfg.define("tiff-tests", "OFF");
        tiff_cfg.define("tiff-docs", "OFF");
        tiff_cfg.define("tiff-contrib", "OFF");
        tiff_cfg.define("tiff-static", "ON");
        tiff_cfg.define("libdeflate", "OFF");
        tiff_cfg.define("zstd", "OFF");
        tiff_cfg.define("lzma", "OFF");
        tiff_cfg.define("webp", "OFF");
        tiff_cfg.define("jpeg", "OFF");
        tiff_cfg.define("jbig", "OFF");
        tiff_cfg.define("lerc", "OFF");

        if let Some((zlib_root, zlib_include, _, zlib_library, _)) = &zlib_paths {
            tiff_cfg.define("ZLIB_ROOT", zlib_root.display().to_string());
            tiff_cfg.define("ZLIB_INCLUDE_DIR", zlib_include.display().to_string());
            tiff_cfg.define("ZLIB_LIBRARY", zlib_library.display().to_string());
        }

        let tiff_build = tiff_cfg.build();
        let include = tiff_build.join("include");
        let lib_dir = tiff_build.join("lib");
        let cmake_package_dir = lib_dir.join("cmake").join("tiff");
        if cmake_package_dir.exists() {
            // PROJ's FindTIFF.cmake prefers libtiff's package config when it is present.
            // The vendored config exports TIFF::tiff with a ZLIB::ZLIB dependency, but
            // does not make that imported target available to PROJ's configure step.
            // Removing it lets FindTIFF use the explicit TIFF_INCLUDE_DIR/TIFF_LIBRARY
            // values we pass below.
            fs::remove_dir_all(cmake_package_dir)?;
        }

        (Some(include), Some(lib_dir))
    } else {
        eprintln!("feature 'tiff' disabled — skipping libtiff build");
        (None, None)
    };

    let path = format!("PROJSRC/proj-{MINIMUM_PROJ_VERSION}.tar.gz");
    let tar_gz = File::open(path)?;
    let tar = GzDecoder::new(tar_gz);
    let mut archive = Archive::new(tar);
    archive.unpack(out_path.join("PROJSRC/proj"))?;
    let mut config =
        cmake::Config::new(out_path.join(format!("PROJSRC/proj/proj-{MINIMUM_PROJ_VERSION}")));
    config.define("ANDROID_STL", "c++_static");
    config.define("APP_STL", "c++_static");
    config.define("BUILD_SHARED_LIBS", "OFF");
    config.define("BUILD_TESTING", "OFF");
    config.define("BUILD_CCT", "OFF");
    config.define("BUILD_CS2CS", "OFF");
    config.define("BUILD_GEOD", "OFF");
    config.define("BUILD_GIE", "OFF");
    config.define("BUILD_PROJ", "OFF");
    config.define("BUILD_PROJINFO", "OFF");
    config.define("BUILD_PROJSYNC", "OFF");
    config.define("ENABLE_CURL", "OFF");

    // we check here whether or not these variables are set by cargo
    // if they are set, `libsqlite3-sys` was built with the bundled feature
    // enabled, which in turn allows us to rely on the built libsqlite3 version
    // and link it statically
    //
    // If these are not set, it's necessary that libsqlite3 exists on the build system
    // in a location accessible by cmake
    if let Ok(sqlite_include) = std::env::var("DEP_SQLITE3_INCLUDE") {
        config.define("SQLITE3_INCLUDE_DIR", sqlite_include);
    }
    if let Ok(sqlite_lib_dir) = std::env::var("DEP_SQLITE3_LIB_DIR") {
        config.define("SQLITE3_LIBRARY", format!("{sqlite_lib_dir}/libsqlite3.a",));
    }

    if let Some((zlib_root, zlib_include, _, zlib_library, _)) = &zlib_paths {
        config.define("ZLIB_ROOT", zlib_root.display().to_string());
        config.define("ZLIB_INCLUDE_DIR", zlib_include.display().to_string());
        config.define("ZLIB_LIBRARY", zlib_library.display().to_string());

        config.define("Z_INCLUDE_DIR", zlib_include.display().to_string());
        config.define("Z_LIBRARY", zlib_library.display().to_string());

    }

    if let (Some(tiff_inc), Some(tiff_lib)) = (&tiff_include, &tiff_lib_dir) {
        let target = env::var("TARGET").unwrap_or_default();
        eprintln!("enabling TIFF support in PROJ build");
        config.define("ENABLE_TIFF", "ON");
        config.define("TIFF_INCLUDE_DIR", tiff_inc.display().to_string());

        let tiff_library = if target.contains("android") {
            tiff_lib.join("libtiff.a")
        } else if target.contains("windows") {
            tiff_lib.join("tiff.lib")
        } else {
            tiff_lib.join("libtiff.a")
        };

        config.define("TIFF_LIBRARY", tiff_library.display().to_string());

    } else {
        eprintln!("disabling TIFF support in PROJ build");
        config.define("ENABLE_TIFF", "OFF");
    }

    if cfg!(target_env = "msvc") {
        // rust links the release MVSC runtime
        // also for debug builds. If we let
        // cmake choose debug/release builds
        // based on the underlying cargo build
        // version that results in linker errors
        config.profile("Release");
    }

    let proj = config.build();
    // Tell cargo to tell rustc to link libproj, and where to find it
    // libproj will be built in $OUT_DIR/lib

    //proj likes to create proj_d when configured as debug and on MSVC, so link to that one if it exists
    println!(
        "cargo:rustc-link-search=native={}",
        proj.join("lib").display()
    );
    if let Some(tiff_lib) = &tiff_lib_dir {
        println!("cargo:rustc-link-search=native={}", tiff_lib.display());
    }
    if let Some((_, _, zlib_lib_dir, _, _)) = &zlib_paths {
        println!("cargo:rustc-link-search=native={}", zlib_lib_dir.display());
    }

    // Static archives are order-sensitive on Android/Linux linkers. Emit
    // consumers first and their dependencies after them, otherwise libtiff can
    // leave zlib symbols such as deflateParams unresolved in the final cdylib.
    if proj.join("lib").join("proj_d.lib").exists() {
        println!("cargo:rustc-link-lib=static=proj_d");
    } else {
        println!("cargo:rustc-link-lib=static=proj");
    }
    if tiff_lib_dir.is_some() {
        println!("cargo:rustc-link-lib=static=tiff");
    }
    if let Some((_, _, _, _, zlib_link_name)) = &zlib_paths {
        println!("cargo:rustc-link-lib=static={zlib_link_name}");
    }

    // This is producing a warning - this directory doesn't exist (on aarch64 anyway)
    println!(
        "cargo:rustc-link-search={}",
        &out_path.join("lib64").display()
    );
    println!(
        "cargo:rustc-link-search={}",
        &out_path.join("build/lib").display()
    );

    Ok(proj.join("include"))
}
