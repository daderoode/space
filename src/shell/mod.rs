pub mod completions;

const INIT_ZSH: &str = include_str!("_space_init.zsh");

pub fn print_init(shell: &str) -> anyhow::Result<()> {
    match shell {
        "zsh" => {
            // Wrapper function
            print!("{}", INIT_ZSH);
            // Inline completion function
            print!("{}", completions::generate_zsh());
            Ok(())
        }
        other => anyhow::bail!("unsupported shell: {}. Only 'zsh' is supported.", other),
    }
}

pub fn print_completions(shell: &str) -> anyhow::Result<()> {
    match shell {
        "zsh" => {
            print!("{}", completions::generate_zsh());
            Ok(())
        }
        other => anyhow::bail!("unsupported shell: {}. Only 'zsh' is supported.", other),
    }
}
