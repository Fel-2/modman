fn main() {
    for g in modeman_core::game::detect_all() {
        println!("{:<28} {}", g.spec.name, g.path.display());
    }
}
