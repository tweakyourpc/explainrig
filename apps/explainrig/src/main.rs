//! Explainrig: documentation in, explainer video out. The window by default; commands for scripts.

mod cli;
mod gui;
mod project;

const COMMANDS: &[&str] = &[
    "help",
    "-h",
    "--help",
    "--version",
    "version",
    "doctor",
    "inspect",
    "import",
    "show",
    "approve",
    "film",
    "prepare",
    "render",
    "auto",
];

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.first() {
        Some(a) if COMMANDS.contains(&a.as_str()) => cli::main(&args),
        Some(a) if a.starts_with('-') && a != "--scene" => cli::main(&args),
        _ => {
            let scene = args
                .iter()
                .position(|a| a == "--scene")
                .and_then(|i| args.get(i + 1))
                .cloned();
            let path = args
                .iter()
                .enumerate()
                .find(|(i, a)| {
                    !a.starts_with("--")
                        && (*i == 0 || args.get(i - 1).is_none_or(|p| p != "--scene"))
                })
                .map(|(_, a)| std::path::PathBuf::from(a));
            gui::run(path, scene)
        }
    };
    std::process::exit(code);
}
