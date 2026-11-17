use std::process::Command;

fn main() {
    embuild::espidf::sysenv::output();

    slint_build::compile_with_config(
        "res/ui/main.slint",
        slint_build::CompilerConfiguration::new()
            .embed_resources(slint_build::EmbedResourcesKind::EmbedForSoftwareRenderer),
    )
    .unwrap();

    // Get git commit hash
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    let output = Command::new("git")
        .args(&["rev-parse", "HEAD"])
        .output()
        .expect("git command failed");
    let commit_hash = str::from_utf8(&output.stdout)
        .expect("git output invalid utf-8")
        .trim();
    println!("cargo:rustc-env=GIT_COMMIT={}", commit_hash);

    // Version string shown to the user: "<version>.mod-<commit date> (<short hash>)", e.g.
    // "1.0.3.mod-20261101 (a60e14f)". "-dirty" is appended to the hash when built with
    // uncommitted changes.
    println!("cargo:rerun-if-changed=../../.git/refs/heads");
    println!("cargo:rerun-if-changed=../../.git/packed-refs");
    println!("cargo:rerun-if-changed=../../.git/index");
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=res");
    let git = |args: &[&str]| -> String {
        let output = Command::new("git").args(args).output().expect("git command failed");
        String::from_utf8(output.stdout).expect("git output invalid utf-8").trim().to_string()
    };
    let commit_date = git(&["log", "-1", "--format=%cd", "--date=format:%Y%m%d"]);
    let short_hash = git(&["rev-parse", "--short=7", "HEAD"]);
    let dirty = !git(&["status", "--porcelain", "--untracked-files=no"]).is_empty();
    println!(
        "cargo:rustc-env=FIRMWARE_DISPLAY_VERSION={}.mod-{} ({}{})",
        env!("CARGO_PKG_VERSION"),
        commit_date,
        short_hash,
        if dirty { "-dirty" } else { "" }
    );

    // Builds with uncommitted changes are test builds: they record extra diagnostics in the
    // crash report (see crash_handler).
    println!("cargo:rustc-check-cfg=cfg(diagnostics)");
    if dirty {
        println!("cargo:rustc-cfg=diagnostics");
    }

    // Cartridge music data (scripts/gbs/music_db.py), embedded when it's there: it holds bytes
    // from GBS rips, so it isn't in git. Without it the firmware has an empty table.
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("cart-music.db");
    println!("cargo:rerun-if-changed=res/cart-music.db");
    match std::fs::read("res/cart-music.db") {
        Ok(data) => std::fs::write(&out, data).unwrap(),
        Err(_) => std::fs::write(&out, b"").unwrap(),
    }

    println!("cargo:rustc-link-arg=-Wl,--wrap=esp_panic_handler");
}
