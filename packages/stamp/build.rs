fn main() {
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-env-changed=GIT_HASH");
    println!("cargo:rustc-env=GIT_HASH={}", subbit_build_info::git_hash());
}
