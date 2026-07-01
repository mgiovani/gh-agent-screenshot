use chrono::Utc;

use crate::error::Result;
use crate::git_data_api::GitHubClient;

pub enum PruneMode {
    DryRun,
    Confirm,
}

pub struct PruneReport {
    pub stale_lines: Vec<String>,
    pub deleted_count: usize,
    pub summary: String,
}

pub async fn run_prune(
    client: &GitHubClient,
    owner: &str,
    repo: &str,
    older_than_days: u64,
    mode: PruneMode,
) -> Result<PruneReport> {
    let refs = client.list_refs(owner, repo, "uploads/").await?;

    if refs.is_empty() {
        return Ok(PruneReport {
            stale_lines: vec![],
            deleted_count: 0,
            summary: "No stale upload refs found.".into(),
        });
    }

    let threshold = older_than_days as i64;
    let mut stale = Vec::new();
    for info in refs {
        let date = client
            .get_commit_date(owner, repo, &info.object_sha)
            .await?;
        let age_days = (Utc::now() - date).num_days();
        if age_days >= threshold {
            stale.push((info, age_days));
        }
    }

    if stale.is_empty() {
        return Ok(PruneReport {
            stale_lines: vec![],
            deleted_count: 0,
            summary: "No stale upload refs found.".into(),
        });
    }

    let stale_lines: Vec<String> = stale
        .iter()
        .map(|(info, age_days)| {
            format!(
                "Upload ref {} is {} days old and would be deleted.",
                info.ref_name, age_days
            )
        })
        .collect();

    match mode {
        PruneMode::DryRun => Ok(PruneReport {
            stale_lines,
            deleted_count: 0,
            summary: String::new(),
        }),
        PruneMode::Confirm => {
            let mut deleted = 0usize;
            for (info, _) in &stale {
                let ref_no_prefix = info
                    .ref_name
                    .strip_prefix("refs/")
                    .unwrap_or(&info.ref_name);
                client.delete_ref(owner, repo, ref_no_prefix).await?;
                deleted += 1;
            }
            Ok(PruneReport {
                stale_lines: vec![],
                deleted_count: deleted,
                summary: format!("Deleted {} stale upload ref(s).", deleted),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn client(server: &MockServer) -> GitHubClient {
        GitHubClient::with_base_url("t".into(), server.uri())
    }

    /// Mount the two-ref fixture: old ref (200 days) and new ref (10 days).
    async fn mount_two_refs(server: &MockServer) {
        Mock::given(method("GET"))
            .and(path("/repos/o/r/git/matching-refs/uploads/"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([
                {
                    "ref": "refs/uploads/issue/old",
                    "object": {"sha": "old123", "type": "commit", "url": "u"}
                },
                {
                    "ref": "refs/uploads/issue/new",
                    "object": {"sha": "new456", "type": "commit", "url": "u"}
                }
            ])))
            .mount(server)
            .await;

        let old_date = (Utc::now() - Duration::days(200)).to_rfc3339();
        Mock::given(method("GET"))
            .and(path("/repos/o/r/git/commits/old123"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "sha": "old123",
                "committer": {"date": old_date, "name": "x", "email": "y"},
                "message": "m",
                "tree": {"sha": "t", "url": "u"},
                "parents": []
            })))
            .mount(server)
            .await;

        let new_date = (Utc::now() - Duration::days(10)).to_rfc3339();
        Mock::given(method("GET"))
            .and(path("/repos/o/r/git/commits/new456"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "sha": "new456",
                "committer": {"date": new_date, "name": "x", "email": "y"},
                "message": "m",
                "tree": {"sha": "t", "url": "u"},
                "parents": []
            })))
            .mount(server)
            .await;
    }

    #[tokio::test]
    async fn dry_run_lists_only_stale_refs() {
        let server = MockServer::start().await;
        mount_two_refs(&server).await;

        // No DELETE must fire in dry-run mode.
        Mock::given(method("DELETE"))
            .respond_with(ResponseTemplate::new(204))
            .expect(0)
            .mount(&server)
            .await;

        let report = run_prune(&client(&server), "o", "r", 90, PruneMode::DryRun)
            .await
            .unwrap();

        assert_eq!(report.stale_lines.len(), 1);
        assert!(report.stale_lines[0].contains("refs/uploads/issue/old"));
        assert!(report.stale_lines[0].contains("200 days"));
        assert_eq!(report.deleted_count, 0);
    }

    #[tokio::test]
    async fn confirm_deletes_only_stale_refs() {
        let server = MockServer::start().await;
        mount_two_refs(&server).await;

        // Only the stale ref's DELETE should be called.
        Mock::given(method("DELETE"))
            .and(path("/repos/o/r/git/refs/uploads/issue/old"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;

        // The recent ref must not be deleted.
        Mock::given(method("DELETE"))
            .and(path("/repos/o/r/git/refs/uploads/issue/new"))
            .respond_with(ResponseTemplate::new(204))
            .expect(0)
            .mount(&server)
            .await;

        let report = run_prune(&client(&server), "o", "r", 90, PruneMode::Confirm)
            .await
            .unwrap();

        assert_eq!(report.deleted_count, 1);
        assert_eq!(report.summary, "Deleted 1 stale upload ref(s).");
    }

    #[tokio::test]
    async fn empty_list_prints_none_sentence() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/o/r/git/matching-refs/uploads/"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .mount(&server)
            .await;

        let report = run_prune(&client(&server), "o", "r", 90, PruneMode::DryRun)
            .await
            .unwrap();

        assert!(report.stale_lines.is_empty());
        assert_eq!(report.deleted_count, 0);
        assert_eq!(report.summary, "No stale upload refs found.");
    }
}
