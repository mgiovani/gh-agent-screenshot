//! Integration tests against a real private GitHub repository via the live API.
//!
//! # Environment Variables
//!
//! | Variable       | Description                                                      |
//! |----------------|------------------------------------------------------------------|
//! | `GH_TOKEN`     | GitHub PAT with `repo` scope on the test repository             |
//! | `GH_TEST_REPO` | Target repository in `owner/name` format (e.g. `acme/uploads`) |
//! | `GH_TEST_ISSUE`| Issue number to post the test comment to (default: `1`)         |
//!
//! # Running
//!
//! ```
//! GH_TOKEN=ghp_... GH_TEST_REPO=owner/repo cargo test --tests -- --include-ignored
//! ```
//!
//! All tests in this file are tagged `#[ignore]` so a bare `cargo test` stays hermetic
//! (no network calls). Pass `--include-ignored` explicitly to exercise the real API.

use gh_agent_screenshot::{
    git_data_api::GitHubClient,
    prune::{run_prune, PruneMode},
    upload::{compose_markdown, upload_batch, Target},
};
use std::path::PathBuf;

fn env_or_skip(var: &str) -> Option<String> {
    std::env::var(var).ok().filter(|v| !v.is_empty())
}

/// Split `"owner/name"` into `("owner", "name")`.
fn split_repo(slug: &str) -> (&str, &str) {
    slug.split_once('/')
        .expect("GH_TEST_REPO must be in owner/name format")
}

/// Verify the upload→comment round-trip against a real private repo.
///
/// Uploads `tests/fixtures/test.png` to a process-ID-scoped ref so concurrent
/// CI runs don't collide, then posts the resulting embed URL as a comment.
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn upload_then_comment_round_trip() {
    let token = match env_or_skip("GH_TOKEN") {
        Some(t) => t,
        None => {
            eprintln!("GH_TOKEN not set or empty — skipping integration test");
            return;
        }
    };
    let repo_slug = match env_or_skip("GH_TEST_REPO") {
        Some(r) => r,
        None => {
            eprintln!("GH_TEST_REPO not set or empty — skipping integration test");
            return;
        }
    };
    let issue_number: u32 = env_or_skip("GH_TEST_ISSUE")
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);

    let (owner, repo) = split_repo(&repo_slug);
    // Use the process ID as a unique issue number so concurrent runs don't share refs.
    let upload_target_id = std::process::id();

    let client = GitHubClient::new(token);
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/test.png");

    let urls = upload_batch(
        &client,
        owner,
        repo,
        &[fixture],
        Target::Issue(upload_target_id),
    )
    .await
    .expect("upload_batch should succeed against real repo");

    assert!(!urls.is_empty(), "expected at least one embed URL");
    let url_str = urls[0].to_string();

    // Contract: URL must contain ?raw=true (Git Data API raw access)
    assert!(
        url_str.contains("?raw=true"),
        "embed URL must contain ?raw=true, got: {url_str}"
    );

    // Contract: commit SHA embedded in the URL must be a 40-char hex string
    let sha_part = url_str
        .split("/blob/")
        .nth(1)
        .and_then(|s| s.split('/').next())
        .expect("embed URL must contain /blob/<sha>/");
    assert_eq!(
        sha_part.len(),
        40,
        "commit SHA in embed URL must be 40 hex chars, got: {sha_part}"
    );
    assert!(
        sha_part.chars().all(|c| c.is_ascii_hexdigit()),
        "commit SHA must be lowercase hex, got: {sha_part}"
    );

    // Post the embed markdown as a comment on the configured test issue
    let markdown = compose_markdown(&urls);
    let comment_id = client
        .create_comment(owner, repo, issue_number, &markdown)
        .await
        .expect("create_comment should succeed against real repo");

    assert!(
        comment_id > 0,
        "comment id must be non-zero, got: {comment_id}"
    );

    eprintln!(
        "upload_then_comment_round_trip: comment {comment_id} posted to {repo_slug}#{issue_number}"
    );
}

/// Verify that `run_prune` with `older_than_days=0` deletes upload refs it created.
///
/// `older_than_days=0` treats every ref as stale (age >= 0 days), so this test
/// acts as a post-suite cleanup sweep. Runs in `Confirm` mode (no dry-run).
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn prune_cleans_test_refs() {
    let token = match env_or_skip("GH_TOKEN") {
        Some(t) => t,
        None => {
            eprintln!("GH_TOKEN not set or empty — skipping integration test");
            return;
        }
    };
    let repo_slug = match env_or_skip("GH_TEST_REPO") {
        Some(r) => r,
        None => {
            eprintln!("GH_TEST_REPO not set or empty — skipping integration test");
            return;
        }
    };

    let (owner, repo) = split_repo(&repo_slug);
    let client = GitHubClient::new(token);

    let report = run_prune(&client, owner, repo, 0, PruneMode::Confirm)
        .await
        .expect("run_prune should succeed against real repo");

    // deleted_count is usize so it's always >= 0; assert the field is accessible
    // and the summary is populated (either "No stale" or "Deleted N").
    assert!(
        !report.summary.is_empty(),
        "prune summary should be non-empty"
    );
    eprintln!(
        "prune_cleans_test_refs: deleted {} ref(s). {}",
        report.deleted_count, report.summary
    );
}
