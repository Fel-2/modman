//! Live smoke test: `cargo run -p modeman-platform --example probe`
//! Hits Thunderstore + GameBanana (no auth). mod.io skipped without a key.

use modeman_platform::{gamebanana::GameBanana, thunderstore::Thunderstore, ListSort, ModPlatform};

fn main() {
    let ts = Thunderstore::new().unwrap();
    match ts.list("lethal-company", ListSort::Top) {
        Ok(mods) => {
            println!("[thunderstore] {} mods; top: {}", mods.len(), mods[0].name);
            match ts.files("lethal-company", &mods[0].id) {
                Ok(files) => println!(
                    "[thunderstore] {} file(s); first: {} -> {}",
                    files.len(),
                    files[0].name,
                    files[0].url
                ),
                Err(e) => println!("[thunderstore] files FAILED: {e}"),
            }
        }
        Err(e) => println!("[thunderstore] list FAILED: {e}"),
    }

    let gb = GameBanana::new().unwrap();
    match gb.list("8722", ListSort::Top) {
        Ok(mods) => {
            println!("[gamebanana] {} mods; first: {}", mods.len(), mods[0].name);
            match gb.files("8722", &mods[0].id) {
                Ok(files) if !files.is_empty() => println!(
                    "[gamebanana] {} file(s); first: {} -> {}",
                    files.len(),
                    files[0].name,
                    files[0].url
                ),
                Ok(_) => println!("[gamebanana] mod has no files"),
                Err(e) => println!("[gamebanana] files FAILED: {e}"),
            }
        }
        Err(e) => println!("[gamebanana] list FAILED: {e}"),
    }
}
