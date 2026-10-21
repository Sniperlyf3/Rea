//! Compiles the vendored miniaudio single-header + the duplex C shim when the
//! `miniaudio-backend` feature is enabled. miniaudio loads the platform audio
//! APIs (ALSA/PulseAudio, Core Audio, WASAPI, AAudio/OpenSL) at runtime, so no
//! audio `-dev` packages are required at build time.

fn main() {
    // Build scripts see enabled features via CARGO_FEATURE_<NAME> env vars.
    if std::env::var_os("CARGO_FEATURE_MINIAUDIO_BACKEND").is_none() {
        return;
    }

    cc::Build::new()
        .file("csrc/qm_miniaudio.c")
        .include("csrc")
        .include("vendor/miniaudio")
        .warnings(false)
        .compile("qm_miniaudio");

    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    match target_os.as_str() {
        "linux" => {
            // miniaudio dlopen()s ALSA/PulseAudio; it needs libdl/libm/pthread.
            println!("cargo:rustc-link-lib=dl");
            println!("cargo:rustc-link-lib=m");
            println!("cargo:rustc-link-lib=pthread");
        }
        "android" => {
            // AAudio/OpenSL are dlopen()ed; libc provides dl/pthread on Android.
        }
        "macos" => {
            for fw in ["CoreFoundation", "CoreAudio", "AudioToolbox", "AudioUnit"] {
                println!("cargo:rustc-link-lib=framework={fw}");
            }
        }
        "ios" => {
            for fw in [
                "CoreFoundation",
                "CoreAudio",
                "AudioToolbox",
                "AVFoundation",
            ] {
                println!("cargo:rustc-link-lib=framework={fw}");
            }
        }
        "windows" => {
            // MSVC auto-links via #pragma in miniaudio; ole32 is the core dep for WASAPI.
            println!("cargo:rustc-link-lib=ole32");
        }
        _ => {}
    }

    println!("cargo:rerun-if-changed=csrc/qm_miniaudio.c");
    println!("cargo:rerun-if-changed=csrc/qm_miniaudio.h");
    println!("cargo:rerun-if-changed=vendor/miniaudio/miniaudio.h");
}
