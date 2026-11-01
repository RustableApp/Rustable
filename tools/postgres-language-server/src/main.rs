//! Private stdio entry point for the unmodified upstream language server.
//! Unlike upstream's CLI proxy this never connects to a shared user daemon.
mod completion;

#[tokio::main]
async fn main() {
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("--completion-catalog")) {
        let result = std::env::args_os()
            .nth(2)
            .ok_or_else(|| "A catalog path is required".to_string())
            .and_then(|path| completion::run(std::path::Path::new(&path)));
        if let Err(error) = result {
            eprintln!("Completion helper: {error}");
            std::process::exit(2);
        }
        return;
    }
    let config = std::env::args_os().nth(1).map(std::path::PathBuf::from);
    let Some(config) = config else {
        eprintln!("An isolated configuration file is required");
        std::process::exit(2);
    };
    pgls_lsp::ServerFactory::new(true)
        .create(Some(config))
        .accept(tokio::io::stdin(), tokio::io::stdout())
        .await;
}
