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
    let loaded = repo::load_cache(&cache_path).unwrap();
    assert_eq!(loaded, paths);
}
