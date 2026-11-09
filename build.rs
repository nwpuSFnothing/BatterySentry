use std::{env, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=assets/app.rc");
    println!("cargo:rerun-if-changed=assets/BatterySentry.ico");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let sdk = env::var_os("WindowsSdkDir")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(
                env::var_os("ProgramFiles(x86)").expect("Windows SDK requires ProgramFiles(x86)"),
            )
            .join("Windows Kits/10")
        });
    let mut versions: Vec<_> = std::fs::read_dir(sdk.join("bin"))
        .expect("Install Windows SDK")
        .filter_map(Result::ok)
        .map(|e| e.path().join("x64/rc.exe"))
        .filter(|p| p.is_file())
        .collect();
    versions.sort();
    let rc = versions
        .last()
        .expect("Windows SDK resource compiler rc.exe");
    let output =
        PathBuf::from(env::var_os("OUT_DIR").expect("Cargo OUT_DIR")).join("BatterySentry.res");
    let status = Command::new(rc)
        .arg("/nologo")
        .arg("/Iassets")
        .arg(format!("/fo{}", output.display()))
        .arg("assets/app.rc")
        .status()
        .expect("Run resource compiler");
    assert!(status.success(), "Resource compiler failed");
    println!("cargo:rustc-link-arg={}", output.display());
}
