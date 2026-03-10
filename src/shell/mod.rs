pub mod completions;

pub fn print_completions(shell: &str) -> anyhow::Result<()> {
    match shell {
        "zsh" => {
            print!("{}", completions::generate_zsh());
            Ok(())
        }
        other => anyhow::bail!("unsupported shell: {}. Only 'zsh' is supported.", other),
    }
}
