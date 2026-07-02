//! Live probe: `cargo run -p modeman-nexus --example search [domain] [query]`.
//! Uses the anonymous v2 GraphQL search (no API key needed).

fn main() {
    let mut args = std::env::args().skip(1);
    let domain = args.next().unwrap_or_else(|| "cyberpunk2077".into());
    let query = args.next().unwrap_or_else(|| "weather".into());
    let client = modeman_nexus::NexusClient::new("anonymous").unwrap();
    match client.search(&domain, &query) {
        Ok(mods) => {
            println!("{} result(s) for '{query}' on {domain}:", mods.len());
            for m in mods.iter().take(5) {
                println!("  #{} {} — {} ({})", m.mod_id, m.name, m.author, m.version);
            }
        }
        Err(e) => {
            eprintln!("search failed: {e}");
            std::process::exit(1);
        }
    }
}
