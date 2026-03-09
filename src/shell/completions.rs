use clap::Command;
use clap_complete::{generate as clap_generate, Shell};
use std::io;

pub fn generate(shell: Shell, cmd: &mut Command) {
    clap_generate(shell, cmd, "space", &mut io::stdout());
}
