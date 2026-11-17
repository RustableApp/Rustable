fn main() -> eframe::Result {
    if let Some(status) = rustable::ssh_askpass_helper() { std::process::exit(status); }
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() == Some("mcp") {
        std::process::exit(rustable::run_mcp(args.collect()));
    }
    rustable::run_native()
}
