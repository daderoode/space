use anyhow::Result;
use crate::core::config::SpaceConfig;

pub fn run(name: Option<String>) -> Result<()> {
    match name {
        None => unreachable!("go without name handled in dispatch"),
        Some(n) => {
            let cfg = SpaceConfig::load()?;
            let ws_path = cfg.workspaces.dir.join(&n);
            if ws_path.is_dir() {
                println!("__SPACE_CD__:{}", ws_path.display());
                Ok(())
            } else {
                anyhow::bail!("workspace '{}' not found", n)
            }
        }
    }
}
