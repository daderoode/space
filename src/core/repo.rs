use anyhow::Result;
use nucleo::{Config as NucleoConfig, Utf32Str};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Walk `roots` up to `max_depth` levels, returning paths that contain `.git`.
/// Does not descend into `.git` directories.
pub fn find_repos_in(roots: &[PathBuf], max_depth: u32) -> Vec<PathBuf> {
    let mut repos = Vec::new();
    for root in roots {
        if !root.exists() {
            continue;
        }
        let mut it = WalkDir::new(root).max_depth(max_depth as usize).into_iter();

        let mut found: Vec<PathBuf> = Vec::new();
        loop {
            match it.next() {
                None => break,
                Some(Err(_)) => continue,
                Some(Ok(entry)) => {
                    if entry.file_name() == ".git" && entry.file_type().is_dir() {
                        if let Some(parent) = entry.path().parent() {
                            found.push(parent.to_path_buf());
                        }
                        it.skip_current_dir();
                    }
                }
            }
        }

        // Filter out repos nested inside other repos (submodule pattern)
        let snapshot = found.clone();
        for r in found {
            if !snapshot
                .iter()
                .any(|other| other != &r && r.starts_with(other))
            {
                repos.push(r);
            }
        }
    }
    repos
}

/// Fuzzy-matches repo paths against a query string. Used in tests.
#[allow(dead_code)]
pub fn fuzzy_match(query: &str, repos: &[PathBuf]) -> Vec<PathBuf> {
    if query.is_empty() {
        return repos.to_vec();
    }

    let mut matcher = nucleo::Matcher::new(NucleoConfig::DEFAULT);
    let pattern = nucleo::pattern::Pattern::new(
        query,
        nucleo::pattern::CaseMatching::Smart,
        nucleo::pattern::Normalization::Smart,
        nucleo::pattern::AtomKind::Fuzzy,
    );

    let mut scored: Vec<(u32, PathBuf)> = repos
        .iter()
        .filter_map(|p| {
            let s = p.to_string_lossy();
            let mut buf = Vec::new();
            let haystack = Utf32Str::new(&s, &mut buf);
            pattern
                .score(haystack, &mut matcher)
                .map(|score| (score, p.clone()))
        })
        .collect();

    scored.sort_by(|a, b| b.0.cmp(&a.0));
    scored.into_iter().map(|(_, p)| p).collect()
}

/// Read a newline-delimited cache file of repo paths.
pub fn load_cache(path: &Path) -> Option<Vec<PathBuf>> {
    let content = std::fs::read_to_string(path).ok()?;
    Some(
        content
            .lines()
            .filter(|l| !l.is_empty())
            .map(PathBuf::from)
            .collect(),
    )
}

/// Write repo paths to a newline-delimited cache file.
pub fn save_cache(path: &Path, repos: &[PathBuf]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let content = repos
        .iter()
        .map(|p| p.to_string_lossy())
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(path, content)?;
    Ok(())
}
