use std::fs;
use std::path::PathBuf;

include!("src/cli.inc.rs");

fn main() {
    println!("cargo:rerun-if-changed=src/cli.inc.rs");
    println!("cargo:rerun-if-changed=build.rs");

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR must be set by Cargo"));
    let completions_dir = out_dir.join("completions");
    fs::create_dir_all(&completions_dir).expect("failed to create completions dir in OUT_DIR");

    let manifest_dir = PathBuf::from(
        std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR must be set by Cargo"),
    );
    let profile = std::env::var("PROFILE").expect("PROFILE must be set by Cargo");
    let target_dir = std::env::var("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| manifest_dir.join("target"));
    let target_completions = target_dir.join(&profile).join("completions");
    fs::create_dir_all(&target_completions).expect("failed to create completions dir in target");

    let nix_build_top = std::env::var("NIX_BUILD_TOP").ok();
    let extra_dirs: [PathBuf; 2] = [completions_dir.clone(), target_completions.clone()];

    let mut cmd = cli();

    for shell in [
        clap_complete::Shell::Bash,
        clap_complete::Shell::Zsh,
        clap_complete::Shell::Fish,
    ] {
        for dir in &extra_dirs {
            clap_complete::generate_to(shell, &mut cmd, "cava-bg", dir)
                .expect("failed to generate shell completions");
        }
        if let Some(ref root) = nix_build_top {
            let nix_dir = PathBuf::from(root).join("completions");
            fs::create_dir_all(&nix_dir).expect("failed to create nix completions dir");
            clap_complete::generate_to(shell, &mut cmd, "cava-bg", &nix_dir)
                .expect("failed to generate shell completions for nix");
        }
    }
}
