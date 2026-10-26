fn main() {
    if let Err(error) = dvdforge_desktop::export_bindings() {
        eprintln!("No se pudieron generar los bindings: {error}");
        std::process::exit(1);
    }
    println!("Bindings generados en {}", dvdforge_desktop::BINDINGS_PATH);
}
