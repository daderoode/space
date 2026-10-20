use crate::core::{
    config::SpaceConfig,
    workspace::{self, UncommittedWork, UncommittedWorkError},
};
use anyhow::Result;

/// `space rm <name> --force [--discard-changes]`.
///
/// `--force` means only "do not ask": git's own mechanics are forced as they
/// always were, but uncommitted work in any repo stops the removal with
/// nothing removed (issue #75) unless `--discard-changes` says to throw it
/// away. clap refuses `--discard-changes` without `--force`, because without
/// `--force` the TUI dialog opens, and it asks about uncommitted work itself.
pub fn run(name: &str, force: bool, discard_changes: bool) -> Result<()> {
    if !force {
        anyhow::bail!("use --force to remove a workspace without confirmation, or run without --force to use the interactive TUI");
    }
    let cfg = SpaceConfig::load()?;
    let uncommitted = if discard_changes {
        UncommittedWork::Discard
    } else {
        UncommittedWork::Refuse
    };
    workspace::remove_space(&cfg.workspaces.dir, name, true, uncommitted).map_err(|e| {
        // The core names no flag; the way out in this command's terms is
        // added here, as the report's last line.
        if e.downcast_ref::<UncommittedWorkError>().is_some() {
            anyhow::anyhow!(
                "{}\nto remove it anyway and lose that work: space rm {} --force --discard-changes",
                e,
                shell_word(name)
            )
        } else {
            e
        }
    })?;
    println!("Removed workspace '{}'", name);
    Ok(())
}

/// `name` as one shell word, so the command in the hint can be pasted as it
/// is: bare when it is plain, single-quoted otherwise (space names may hold
/// blanks and other characters a shell would split or expand).
fn shell_word(name: &str) -> String {
    let plain = name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '+' | '@' | ':' | ','));
    if plain {
        name.to_string()
    } else {
        format!("'{}'", name.replace('\'', "'\\''"))
    }
}

#[cfg(test)]
mod tests {
    use super::shell_word;

    #[test]
    fn a_hint_name_is_one_shell_word() {
        assert_eq!(shell_word("feature-x.2"), "feature-x.2");
        assert_eq!(shell_word("my space"), "'my space'");
        assert_eq!(shell_word("it's"), "'it'\\''s'");
        assert_eq!(shell_word("$HOME"), "'$HOME'");
    }
}
