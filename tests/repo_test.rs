use filetime::FileTime;
use space::core::repo;
use std::path::PathBuf;
use tempfile::TempDir;

fn make_git_repo(path: &std::path::Path) {
    std::fs::create_dir_all(path.join(".git")).unwrap();
}

#[test]
fn finds_git_repos_within_root() {
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path().join("my-repo");
    make_git_repo(&repo);

    let found = repo::find_repos_in(&[tmp.path().to_path_buf()], 3);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0], repo);
}

#[test]
fn respects_max_depth() {
    let tmp = TempDir::new().unwrap();
    // depth 4 from root — should not be found at max_depth=2
    let deep = tmp.path().join("a/b/c/d/deep-repo");
    make_git_repo(&deep);

    let found = repo::find_repos_in(&[tmp.path().to_path_buf()], 2);
    assert!(found.is_empty(), "should not find repos beyond max_depth");
}

#[test]
fn does_not_descend_into_git_dirs() {
    // A repo inside a repo (git submodule pattern) — only outer should appear
    let tmp = TempDir::new().unwrap();
    let outer = tmp.path().join("outer");
    make_git_repo(&outer);
    let inner = outer.join("inner");
    make_git_repo(&inner);

    let found = repo::find_repos_in(&[tmp.path().to_path_buf()], 5);
    assert_eq!(found.len(), 1, "should only find the outer repo");
    assert_eq!(found[0], outer);
}

#[test]
fn fuzzy_match_returns_best_matches() {
    let repos = vec![
        PathBuf::from("/work/acme/acme-api"),
        PathBuf::from("/work/acme/acme-web"),
        PathBuf::from("/work/tools/auth-service"),
    ];
    let matches = repo::fuzzy_match("acme-api", &repos);
    assert!(!matches.is_empty());
    assert_eq!(
        matches[0].file_name().unwrap(),
        "acme-api",
        "best match should be first"
    );
}

#[test]
fn fuzzy_match_no_results_for_garbage() {
    let repos = vec![PathBuf::from("/work/acme/acme-api")];
    let matches = repo::fuzzy_match("zzzzzzzzz", &repos);
    assert!(matches.is_empty());
}

#[test]
fn cache_round_trips() {
    let tmp = TempDir::new().unwrap();
    let cache_path = tmp.path().join("repos.cache");
    let paths = vec![PathBuf::from("/work/repo-a"), PathBuf::from("/work/repo-b")];
    repo::save_cache(&cache_path, &paths).unwrap();
    let loaded = repo::load_cache(&cache_path, 3600).unwrap();
    assert_eq!(loaded, paths);
}

#[test]
fn load_cache_returns_none_for_missing_file() {
    let tmp = TempDir::new().unwrap();
    let cache = tmp.path().join("nonexistent.cache");
    let result = repo::load_cache(&cache, 3600);
    assert!(result.is_none());
}

#[test]
fn load_cache_returns_none_for_stale_cache() {
    let tmp = TempDir::new().unwrap();
    let cache = tmp.path().join("repos.cache");
    repo::save_cache(&cache, &[PathBuf::from("/a")]).unwrap();

    // Backdate the file's mtime by 2 hours
    let two_hours_ago = FileTime::from_system_time(
        std::time::SystemTime::now() - std::time::Duration::from_secs(7200),
    );
    filetime::set_file_mtime(&cache, two_hours_ago).unwrap();

    // TTL of 1 hour should reject this cache
    let result = repo::load_cache(&cache, 3600);
    assert!(result.is_none());
}

#[test]
fn fuzzy_match_empty_query_returns_all() {
    let repos = vec![
        PathBuf::from("/work/alpha"),
        PathBuf::from("/work/beta"),
        PathBuf::from("/work/gamma"),
    ];
    let matches = repo::fuzzy_match("", &repos);
    assert_eq!(matches.len(), 3, "empty query should return all repos");
    assert_eq!(matches, repos, "order should be preserved");
}
