pub mod completions;

pub fn print_completions(shell: &str) {
    match shell {
        "zsh" => print!("{}", completions::generate_zsh()),
        other => eprintln!("unsupported shell: {other}. Only 'zsh' is supported."),
    }
}
