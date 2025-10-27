fn main() {
    // Rebuild the binary whenever a migration file is added or modified.
    println!("cargo:rerun-if-changed=migrations");
}
