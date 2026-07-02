//! Live probe: `cargo run -p modeman-platform --example search`.
//! Exercises Thunderstore (client-side) and GameBanana (server-side) search.

use modeman_platform::{gamebanana::GameBanana, thunderstore::Thunderstore, ModPlatform};

fn main() {
    let ts = Thunderstore::new().unwrap();
    match ts.search("cyberpunk2077", "cyber engine") {
        Ok(mods) => {
            println!("thunderstore: {} result(s)", mods.len());
            for m in mods.iter().take(3) {
                println!("  {} — {} ({} dl)", m.id, m.name, m.downloads);
            }
        }
        Err(e) => println!("thunderstore search failed: {e}"),
    }

    let gb = GameBanana::new().unwrap();
    match gb.search("8722", "weather") {
        Ok(mods) => {
            println!("gamebanana: {} result(s)", mods.len());
            for m in mods.iter().take(3) {
                println!("  {} — {} by {}", m.id, m.name, m.author);
            }
        }
        Err(e) => println!("gamebanana search failed: {e}"),
    }
}
