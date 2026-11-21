fn main() {
    if let Err(error) = kinodisc_desktop::export_bindings() {
        eprintln!("No se pudieron generar los bindings: {error}");
        std::process::exit(1);
    }
    println!("Bindings generados en {}", kinodisc_desktop::BINDINGS_PATH);
}
