fn main() -> eframe::Result {
    if let Some(status) = rustable::ssh_askpass_helper() {
        std::process::exit(status);
    }
    #[cfg(unix)]
    {
        let args: Vec<_> = std::env::args().skip(1).collect();
        if args
            .first()
            .is_some_and(|arg| matches!(arg.as_str(), "matrix" | "mcp"))
        {
            if args.len() != 3 || args[1] != "--socket" {
                eprintln!("Usage: rustable matrix|mcp --socket /private/directory/matrix.sock");
                std::process::exit(2);
            }
            if args[0] == "mcp" {
                std::process::exit(rustable::matrix::client(std::path::Path::new(&args[2])));
            }
            return rustable::run_matrix(args[2].clone().into());
        }
    }
    rustable::run_native()
}
