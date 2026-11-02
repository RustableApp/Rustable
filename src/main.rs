fn main() -> eframe::Result {
    if let Some(status) = rustable::ssh_askpass_helper() { std::process::exit(status); }
    rustable::run_native()
}
