//! Rebuild this crate when a migration is added or changed (sqlx embeds them at compile time).

fn main() {
    println!("cargo:rerun-if-changed=migrations");
}
