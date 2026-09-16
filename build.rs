// Stamp the build with the git revision so the CLI and the Python module can
// tell each other apart (`pcb --version`, `flatland.version()`).
fn main() {
    let git = std::process::Command::new("git")
        .args(["describe", "--always", "--dirty", "--tags"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=FLATLAND_VERSION={} (git {git})", env!("CARGO_PKG_VERSION"));
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/refs/heads");
    println!("cargo:rerun-if-changed=.git/index");
}
