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

    // Version string shown to the user: "<version>-custom <build date>", e.g.
    // "1.0.3-custom 31-Oct-2026". The date is refreshed whenever sources change.
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=res");
    let output = Command::new("date")
        .arg("+%d-%b-%Y")
        .env("LC_ALL", "C")
        .output()
        .expect("date command failed");
    let build_date = str::from_utf8(&output.stdout)
        .expect("date output invalid utf-8")
        .trim();
    println!(
        "cargo:rustc-env=FIRMWARE_DISPLAY_VERSION={}-custom {}",
        env!("CARGO_PKG_VERSION"),
        build_date
    );

    println!("cargo:rustc-link-arg=-Wl,--wrap=esp_panic_handler");
}
