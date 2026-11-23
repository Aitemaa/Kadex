use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

const PROTOCOLS: &[(&str, &str)] = &[
    ("xdg-shell.xml", "xdg-shell-client-protocol"),
    (
        "xdg-decoration-unstable-v1.xml",
        "xdg-decoration-unstable-v1-client-protocol",
    ),
    (
        "fractional-scale-v1.xml",
        "fractional-scale-v1-client-protocol",
    ),
    ("viewporter.xml", "viewporter-client-protocol"),
    (
        "wp-primary-selection-unstable-v1.xml",
        "wp-primary-selection-unstable-v1-client-protocol",
    ),
    (
        "text-input-unstable-v3.xml",
        "text-input-unstable-v3-client-protocol",
    ),
    (
        "wlr-layer-shell-unstable-v1.xml",
        "wlr-layer-shell-unstable-v1-client-protocol",
    ),
];

fn main() {
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os != "linux" {
        return;
    }

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let proto_dir = manifest_dir.join("protocols");
    let c_dir = manifest_dir.join("c");

    println!("cargo:rerun-if-changed=c/wl_shim.h");
    println!("cargo:rerun-if-changed=c/wl_shim.c");
    for (xml, _) in PROTOCOLS {
        println!("cargo:rerun-if-changed=protocols/{xml}");
    }

    let mut c_sources = Vec::new();
    for (xml, base) in PROTOCOLS {
        let xml_path = proto_dir.join(xml);
        let h_path = out_dir.join(format!("{base}.h"));
        let c_path = out_dir.join(format!("{base}.c"));

        run_scanner("client-header", &xml_path, &h_path);
        run_scanner("private-code", &xml_path, &c_path);
        c_sources.push(c_path);
    }
    c_sources.push(c_dir.join("wl_shim.c"));

    // Query pkg-config for include flags
    let pkg_cflags = pkg_config_args(&[
        "--cflags",
        "wayland-client",
        "wayland-cursor",
        "xkbcommon",
        "pangocairo",
        "pango",
        "cairo",
    ]);

    let cc = env::var("CC").unwrap_or_else(|_| "cc".to_string());
    let mut objects = Vec::new();
    for (idx, src) in c_sources.iter().enumerate() {
        let obj = out_dir.join(format!("obj_{idx}.o"));
        let mut cmd = Command::new(&cc);
        cmd.arg("-Os")
            .arg("-fPIC")
            .arg("-ffunction-sections")
            .arg("-fdata-sections")
            .arg("-D_GNU_SOURCE")
            .arg("-I")
            .arg(&out_dir)
            .arg("-I")
            .arg(&c_dir);
        for flag in &pkg_cflags {
            cmd.arg(flag);
        }
        cmd.arg("-c").arg(src).arg("-o").arg(&obj);
        let status = cmd.status().expect("failed to invoke C compiler");
        assert!(status.success(), "C compilation failed for {}", src.display());
        objects.push(obj);
    }

    let archive = out_dir.join("libwl_shim.a");
    let mut ar = Command::new("ar");
    ar.arg("rcs").arg(&archive);
    for obj in &objects {
        ar.arg(obj);
    }
    let status = ar.status().expect("failed to invoke ar");
    assert!(status.success(), "ar failed");

    println!("cargo:rustc-link-search=native={}", out_dir.display());
    println!("cargo:rustc-link-lib=static=wl_shim");

    for lib in [
        "wayland-client",
        "wayland-cursor",
        "xkbcommon",
        "pangocairo-1.0",
        "pango-1.0",
        "cairo",
        "gobject-2.0",
        "glib-2.0",
    ] {
        println!("cargo:rustc-link-lib=dylib={lib}");
    }
}

fn run_scanner(mode: &str, xml: &Path, out: &Path) {
    let status = Command::new("wayland-scanner")
        .arg(mode)
        .arg(xml)
        .arg(out)
        .status()
        .expect("failed to run wayland-scanner");
    assert!(
        status.success(),
        "wayland-scanner {mode} failed for {}",
        xml.display()
    );
}

fn pkg_config_args(args: &[&str]) -> Vec<String> {
    let out = Command::new("pkg-config")
        .args(args)
        .output()
        .expect("failed to invoke pkg-config");
    assert!(out.status.success(), "pkg-config failed");
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .map(String::from)
        .collect()
}
