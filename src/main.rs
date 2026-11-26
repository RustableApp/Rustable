// Release builds are GUI apps on Windows: no console window behind Rustable.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
fn main() -> eframe::Result {
    if let Some(status) = rustable::ssh_askpass_helper() {
        std::process::exit(status);
    }
    #[cfg(unix)]
    {
        let args: Vec<_> = std::env::args().skip(1).collect();
        if args
            .first()
            .is_some_and(|arg| matches!(arg.as_str(), "matrix" | "matrix-demo" | "mcp"))
        {
            if args.len() != 3 || args[1] != "--socket" {
                eprintln!(
                    "Usage: rustable matrix|matrix-demo|mcp --socket /private/directory/matrix.sock"
                );
                std::process::exit(2);
            }
            if args[0] == "mcp" {
                std::process::exit(rustable::matrix::client(std::path::Path::new(&args[2])));
            }
            return if args[0] == "matrix-demo" {
                rustable::run_matrix_demo(args[2].clone().into())
            } else {
                rustable::run_matrix(args[2].clone().into())
            };
        }
    }
    rustable::run_native()
}
