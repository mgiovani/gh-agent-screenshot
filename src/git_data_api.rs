use base64::{engine::general_purpose::STANDARD, Engine as _};
use chrono::{DateTime, Utc};
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION, USER_AGENT};
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

const GITHUB_API: &str = "https://api.github.com";
const API_VERSION: &str = "2022-11-28";

pub struct GitHubClient {
    pub http: reqwest::Client,
    pub base_url: String,
    pub token: String,
}

impl GitHubClient {
    pub fn new(token: String) -> Self {
        Self::with_base_url(token, GITHUB_API.to_string())
    }

    pub fn with_base_url(token: String, base_url: String) -> Self {
        Self {
            http: reqwest::Client::new(),
            base_url,
            token,
        }
    }

    fn auth_headers(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", self.token)).unwrap(),
        );
        headers.insert(
            ACCEPT,
            HeaderValue::from_static("application/vnd.github+json"),
        );
        headers.insert(
            "x-github-api-version",
            HeaderValue::from_static(API_VERSION),
        );
        headers.insert(USER_AGENT, HeaderValue::from_static("gh-agent-screenshot"));
        headers
    }

    async fn check_response(resp: reqwest::Response) -> Result<reqwest::Response> {
        let status = resp.status();
        if status.is_success() {
            return Ok(resp);
        }
        match status.as_u16() {
            401 => Err(Error::AuthFailure),
            403 => {
                let rate_remaining = resp
                    .headers()
                    .get("x-ratelimit-remaining")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("1");
                if rate_remaining == "0" {
                    Err(Error::RateLimited { retry_after: None })
                } else {
                    let body = resp.text().await.unwrap_or_default();
                    if body.contains("rate limit exceeded") {
                        Err(Error::RateLimited { retry_after: None })
                    } else {
                        Err(Error::ApiError {
                            status: 403,
                            message: body,
                        })
                    }
                }
            }
            404 => {
                let body = resp.text().await.unwrap_or_default();
                Err(Error::ApiError {
                    status: 404,
                    message: body,
                })
            }
            409 => Err(Error::EmptyRepo),
            422 => {
                let body = resp.text().await.unwrap_or_default();
                Err(Error::ApiError {
                    status: 422,
                    message: body,
                })
            }
            429 => {
                let retry_after = resp
                    .headers()
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.parse::<u64>().ok());
                Err(Error::RateLimited { retry_after })
            }
            code => {
                let body = resp.text().await.unwrap_or_default();
                Err(Error::ApiError {
                    status: code,
                    message: body,
                })
            }
        }
    }

    pub async fn create_blob(
        &self,
        owner: &str,
        repo: &str,
        content_bytes: &[u8],
    ) -> Result<String> {
        #[derive(Serialize)]
        struct Body {
            content: String,
            encoding: &'static str,
        }
        #[derive(Deserialize)]
        struct Resp {
            sha: String,
        }
        let body = Body {
            content: STANDARD.encode(content_bytes),
            encoding: "base64",
        };
        let resp = self
            .http
            .post(format!(
                "{}/repos/{}/{}/git/blobs",
                self.base_url, owner, repo
            ))
            .headers(self.auth_headers())
            .json(&body)
            .send()
            .await?;
        let resp = Self::check_response(resp).await?;
        let r: Resp = resp.json().await?;
        Ok(r.sha)
    }

    pub async fn create_tree(
        &self,
        owner: &str,
        repo: &str,
        base_tree: Option<&str>,
        entries: &[TreeEntry],
    ) -> Result<String> {
        #[derive(Serialize)]
        struct TreeItem<'a> {
            path: &'a str,
            mode: &'static str,
            #[serde(rename = "type")]
            kind: &'static str,
            sha: &'a str,
        }
        #[derive(Serialize)]
        struct Body<'a> {
            #[serde(skip_serializing_if = "Option::is_none")]
            base_tree: Option<&'a str>,
            tree: Vec<TreeItem<'a>>,
        }
        #[derive(Deserialize)]
        struct Resp {
            sha: String,
        }
        let tree_items: Vec<TreeItem> = entries
            .iter()
            .map(|e| TreeItem {
                path: &e.path,
                mode: "100644",
                kind: "blob",
                sha: &e.sha,
            })
            .collect();
        let body = Body {
            base_tree,
            tree: tree_items,
        };
        let resp = self
            .http
            .post(format!(
                "{}/repos/{}/{}/git/trees",
                self.base_url, owner, repo
            ))
            .headers(self.auth_headers())
            .json(&body)
            .send()
            .await?;
        let resp = Self::check_response(resp).await?;
        let r: Resp = resp.json().await?;
        Ok(r.sha)
    }

    pub async fn create_commit(
        &self,
        owner: &str,
        repo: &str,
        message: &str,
        tree_sha: &str,
        parents: &[String],
    ) -> Result<String> {
        #[derive(Serialize)]
        struct Body<'a> {
            message: &'a str,
            tree: &'a str,
            parents: &'a [String],
        }
        #[derive(Deserialize)]
        struct Resp {
            sha: String,
        }
        let body = Body {
            message,
            tree: tree_sha,
            parents,
        };
        let resp = self
            .http
            .post(format!(
                "{}/repos/{}/{}/git/commits",
                self.base_url, owner, repo
            ))
            .headers(self.auth_headers())
            .json(&body)
            .send()
            .await?;
        let resp = Self::check_response(resp).await?;
        let r: Resp = resp.json().await?;
        Ok(r.sha)
    }

    pub async fn get_ref(
        &self,
        owner: &str,
        repo: &str,
        ref_no_prefix: &str,
    ) -> Result<Option<RefInfo>> {
        #[derive(Deserialize)]
        struct ObjectSha {
            sha: String,
        }
        #[derive(Deserialize)]
        struct Resp {
            #[serde(rename = "ref")]
            ref_name: String,
            object: ObjectSha,
        }
        let resp = self
            .http
            .get(format!(
                "{}/repos/{}/{}/git/ref/{}",
                self.base_url, owner, repo, ref_no_prefix
            ))
            .headers(self.auth_headers())
            .send()
            .await?;
        if resp.status() == 404 {
            return Ok(None);
        }
        let resp = Self::check_response(resp).await?;
        let r: Resp = resp.json().await?;
        Ok(Some(RefInfo {
            ref_name: r.ref_name,
            object_sha: r.object.sha,
        }))
    }

    pub async fn create_ref(
        &self,
        owner: &str,
        repo: &str,
        full_ref: &str,
        sha: &str,
    ) -> Result<()> {
        #[derive(Serialize)]
        struct Body<'a> {
            #[serde(rename = "ref")]
            full_ref: &'a str,
            sha: &'a str,
        }
        let body = Body { full_ref, sha };
        let resp = self
            .http
            .post(format!(
                "{}/repos/{}/{}/git/refs",
                self.base_url, owner, repo
            ))
            .headers(self.auth_headers())
            .json(&body)
            .send()
            .await?;
        Self::check_response(resp).await?;
        Ok(())
    }

    pub async fn update_ref(
        &self,
        owner: &str,
        repo: &str,
        ref_no_prefix: &str,
        sha: &str,
        force: bool,
    ) -> Result<()> {
        #[derive(Serialize)]
        struct Body<'a> {
            sha: &'a str,
            force: bool,
        }
        let body = Body { sha, force };
        let resp = self
            .http
            .patch(format!(
                "{}/repos/{}/{}/git/refs/{}",
                self.base_url, owner, repo, ref_no_prefix
            ))
            .headers(self.auth_headers())
            .json(&body)
            .send()
            .await?;
        Self::check_response(resp).await?;
        Ok(())
    }

    pub async fn list_refs(
        &self,
        owner: &str,
        repo: &str,
        prefix_no_prefix: &str,
    ) -> Result<Vec<RefInfo>> {
        #[derive(Deserialize)]
        struct ObjectSha {
            sha: String,
        }
        #[derive(Deserialize)]
        struct Item {
            #[serde(rename = "ref")]
            ref_name: String,
            object: ObjectSha,
        }
        let resp = self
            .http
            .get(format!(
                "{}/repos/{}/{}/git/matching-refs/{}",
                self.base_url, owner, repo, prefix_no_prefix
            ))
            .headers(self.auth_headers())
            .send()
            .await?;
        let resp = Self::check_response(resp).await?;
        let items: Vec<Item> = resp.json().await?;
        Ok(items
            .into_iter()
            .map(|i| RefInfo {
                ref_name: i.ref_name,
                object_sha: i.object.sha,
            })
            .collect())
    }

    pub async fn get_commit_tree_sha(
        &self,
        owner: &str,
        repo: &str,
        commit_sha: &str,
    ) -> Result<String> {
        #[derive(Deserialize)]
        struct TreeInfo {
            sha: String,
        }
        #[derive(Deserialize)]
        struct CommitResp {
            tree: TreeInfo,
        }
        let resp = self
            .http
            .get(format!(
                "{}/repos/{}/{}/git/commits/{}",
                self.base_url, owner, repo, commit_sha
            ))
            .headers(self.auth_headers())
            .send()
            .await?;
        let resp = Self::check_response(resp).await?;
        let r: CommitResp = resp.json().await?;
        Ok(r.tree.sha)
    }

    pub async fn get_commit_date(
        &self,
        owner: &str,
        repo: &str,
        commit_sha: &str,
    ) -> Result<DateTime<Utc>> {
        #[derive(Deserialize)]
        struct CommitterInfo {
            date: DateTime<Utc>,
        }
        #[derive(Deserialize)]
        struct CommitResp {
            committer: CommitterInfo,
        }
        let resp = self
            .http
            .get(format!(
                "{}/repos/{}/{}/git/commits/{}",
                self.base_url, owner, repo, commit_sha
            ))
            .headers(self.auth_headers())
            .send()
            .await?;
        let resp = Self::check_response(resp).await?;
        let r: CommitResp = resp.json().await?;
        Ok(r.committer.date)
    }

    pub async fn delete_ref(&self, owner: &str, repo: &str, ref_no_prefix: &str) -> Result<()> {
        let resp = self
            .http
            .delete(format!(
                "{}/repos/{}/{}/git/refs/{}",
                self.base_url, owner, repo, ref_no_prefix
            ))
            .headers(self.auth_headers())
            .send()
            .await?;
        // 204 No Content is success; check_response treats all 2xx as Ok
        Self::check_response(resp).await?;
        Ok(())
    }

    /// GitHub routes PR comments through /issues/{n}/comments — same endpoint for both.
    pub async fn create_comment(
        &self,
        owner: &str,
        repo: &str,
        issue_or_pr_number: u32,
        body: &str,
    ) -> Result<u64> {
        #[derive(Serialize)]
        struct Body<'a> {
            body: &'a str,
        }
        #[derive(Deserialize)]
        struct Resp {
            id: u64,
        }
        let b = Body { body };
        let resp = self
            .http
            .post(format!(
                "{}/repos/{}/{}/issues/{}/comments",
                self.base_url, owner, repo, issue_or_pr_number
            ))
            .headers(self.auth_headers())
            .json(&b)
            .send()
            .await?;
        let resp = Self::check_response(resp).await?;
        let r: Resp = resp.json().await?;
        Ok(r.id)
    }

    pub async fn update_comment(
        &self,
        owner: &str,
        repo: &str,
        comment_id: u64,
        body: &str,
    ) -> Result<()> {
        #[derive(Serialize)]
        struct Body<'a> {
            body: &'a str,
        }
        let b = Body { body };
        let resp = self
            .http
            .patch(format!(
                "{}/repos/{}/{}/issues/comments/{}",
                self.base_url, owner, repo, comment_id
            ))
            .headers(self.auth_headers())
            .json(&b)
            .send()
            .await?;
        Self::check_response(resp).await?;
        Ok(())
    }

    pub async fn patch_issue_body(
        &self,
        owner: &str,
        repo: &str,
        issue_number: u32,
        body: &str,
    ) -> Result<()> {
        #[derive(Serialize)]
        struct Body<'a> {
            body: &'a str,
        }
        let b = Body { body };
        let resp = self
            .http
            .patch(format!(
                "{}/repos/{}/{}/issues/{}",
                self.base_url, owner, repo, issue_number
            ))
            .headers(self.auth_headers())
            .json(&b)
            .send()
            .await?;
        Self::check_response(resp).await?;
        Ok(())
    }

    pub async fn patch_pr_body(
        &self,
        owner: &str,
        repo: &str,
        pr_number: u32,
        body: &str,
    ) -> Result<()> {
        #[derive(Serialize)]
        struct Body<'a> {
            body: &'a str,
        }
        let b = Body { body };
        let resp = self
            .http
            .patch(format!(
                "{}/repos/{}/{}/pulls/{}",
                self.base_url, owner, repo, pr_number
            ))
            .headers(self.auth_headers())
            .json(&b)
            .send()
            .await?;
        Self::check_response(resp).await?;
        Ok(())
    }

    pub async fn get_issue_body(
        &self,
        owner: &str,
        repo: &str,
        issue_number: u32,
    ) -> Result<String> {
        #[derive(Deserialize)]
        struct Resp {
            body: Option<String>,
        }
        let resp = self
            .http
            .get(format!(
                "{}/repos/{}/{}/issues/{}",
                self.base_url, owner, repo, issue_number
            ))
            .headers(self.auth_headers())
            .send()
            .await?;
        let resp = Self::check_response(resp).await?;
        let r: Resp = resp.json().await?;
        Ok(r.body.unwrap_or_default())
    }

    pub async fn get_pr_body(&self, owner: &str, repo: &str, pr_number: u32) -> Result<String> {
        #[derive(Deserialize)]
        struct Resp {
            body: Option<String>,
        }
        let resp = self
            .http
            .get(format!(
                "{}/repos/{}/{}/pulls/{}",
                self.base_url, owner, repo, pr_number
            ))
            .headers(self.auth_headers())
            .send()
            .await?;
        let resp = Self::check_response(resp).await?;
        let r: Resp = resp.json().await?;
        Ok(r.body.unwrap_or_default())
    }

    pub async fn get_comment_body(
        &self,
        owner: &str,
        repo: &str,
        comment_id: u64,
    ) -> Result<String> {
        #[derive(Deserialize)]
        struct Resp {
            body: Option<String>,
        }
        let resp = self
            .http
            .get(format!(
                "{}/repos/{}/{}/issues/comments/{}",
                self.base_url, owner, repo, comment_id
            ))
            .headers(self.auth_headers())
            .send()
            .await?;
        let resp = Self::check_response(resp).await?;
        let r: Resp = resp.json().await?;
        Ok(r.body.unwrap_or_default())
    }

    // ponytail: read-then-PATCH is a TOCTOU race under concurrent edits; acceptable for a screenshot tool.

    pub async fn append_issue_body(
        &self,
        owner: &str,
        repo: &str,
        issue_number: u32,
        addition: &str,
    ) -> Result<()> {
        let existing = self.get_issue_body(owner, repo, issue_number).await?;
        self.patch_issue_body(owner, repo, issue_number, &append_body(&existing, addition))
            .await
    }

    pub async fn append_pr_body(
        &self,
        owner: &str,
        repo: &str,
        pr_number: u32,
        addition: &str,
    ) -> Result<()> {
        let existing = self.get_pr_body(owner, repo, pr_number).await?;
        self.patch_pr_body(owner, repo, pr_number, &append_body(&existing, addition))
            .await
    }

    pub async fn append_comment(
        &self,
        owner: &str,
        repo: &str,
        comment_id: u64,
        addition: &str,
    ) -> Result<()> {
        let existing = self.get_comment_body(owner, repo, comment_id).await?;
        self.update_comment(owner, repo, comment_id, &append_body(&existing, addition))
            .await
    }
}

/// Append `addition` after `existing`, blank-line separated. Empty existing → just addition.
pub fn append_body(existing: &str, addition: &str) -> String {
    if existing.trim().is_empty() {
        addition.to_string()
    } else {
        format!("{}\n\n{}", existing.trim_end(), addition)
    }
}

pub struct TreeEntry {
    pub path: String,
    pub sha: String,
}

pub struct RefInfo {
    pub ref_name: String,
    pub object_sha: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use serde_json::json;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn client(server: &MockServer) -> GitHubClient {
        GitHubClient::with_base_url("test-token".into(), server.uri())
    }

    #[tokio::test]
    async fn create_blob_sends_base64_body_and_returns_sha() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/repos/o/r/git/blobs"))
            .and(header("authorization", "Bearer test-token"))
            .and(header("user-agent", "gh-agent-screenshot"))
            .respond_with(
                ResponseTemplate::new(201).set_body_json(json!({"sha": "blobsha", "url": "x"})),
            )
            .expect(1)
            .mount(&server)
            .await;
        let sha = client(&server).create_blob("o", "r", b"hi").await.unwrap();
        assert_eq!(sha, "blobsha");
    }

    #[tokio::test]
    async fn create_blob_body_is_base64_encoded() {
        // "hi" base64 = "aGk="
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/repos/o/r/git/blobs"))
            .and(wiremock::matchers::body_json(
                json!({"content": "aGk=", "encoding": "base64"}),
            ))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({"sha": "abc"})))
            .expect(1)
            .mount(&server)
            .await;
        client(&server).create_blob("o", "r", b"hi").await.unwrap();
    }

    #[tokio::test]
    async fn create_tree_without_base_tree_omits_field() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/repos/o/r/git/trees"))
            .and(wiremock::matchers::body_json(json!({
                "tree": [{"path":"img.png","mode":"100644","type":"blob","sha":"blobsha"}]
            })))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({"sha": "treesha"})))
            .expect(1)
            .mount(&server)
            .await;
        let sha = client(&server)
            .create_tree(
                "o",
                "r",
                None,
                &[TreeEntry {
                    path: "img.png".into(),
                    sha: "blobsha".into(),
                }],
            )
            .await
            .unwrap();
        assert_eq!(sha, "treesha");
    }

    #[tokio::test]
    async fn create_tree_with_base_tree_includes_field() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/repos/o/r/git/trees"))
            .and(wiremock::matchers::body_json(json!({
                "base_tree": "xyz",
                "tree": [{"path":"img.png","mode":"100644","type":"blob","sha":"bs"}]
            })))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({"sha": "ts"})))
            .expect(1)
            .mount(&server)
            .await;
        client(&server)
            .create_tree(
                "o",
                "r",
                Some("xyz"),
                &[TreeEntry {
                    path: "img.png".into(),
                    sha: "bs".into(),
                }],
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn create_commit_empty_parents_serializes_empty_array() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/repos/o/r/git/commits"))
            .and(wiremock::matchers::body_json(
                json!({"message":"msg","tree":"ts","parents":[]}),
            ))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({"sha": "csha"})))
            .expect(1)
            .mount(&server)
            .await;
        let sha = client(&server)
            .create_commit("o", "r", "msg", "ts", &[])
            .await
            .unwrap();
        assert_eq!(sha, "csha");
    }

    #[tokio::test]
    async fn create_commit_with_parent() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/repos/o/r/git/commits"))
            .and(wiremock::matchers::body_json(
                json!({"message":"m","tree":"t","parents":["p1"]}),
            ))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({"sha": "c2"})))
            .expect(1)
            .mount(&server)
            .await;
        client(&server)
            .create_commit("o", "r", "m", "t", &["p1".to_string()])
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn get_ref_returns_none_on_404() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/o/r/git/ref/uploads/issue/1"))
            .respond_with(ResponseTemplate::new(404).set_body_json(json!({"message":"Not Found"})))
            .expect(1)
            .mount(&server)
            .await;
        let result = client(&server)
            .get_ref("o", "r", "uploads/issue/1")
            .await
            .unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn get_ref_returns_some_on_200() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/o/r/git/ref/uploads/issue/1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "ref": "refs/uploads/issue/1",
                "object": {"sha": "abc123", "type": "commit", "url": "x"}
            })))
            .expect(1)
            .mount(&server)
            .await;
        let info = client(&server)
            .get_ref("o", "r", "uploads/issue/1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(info.ref_name, "refs/uploads/issue/1");
        assert_eq!(info.object_sha, "abc123");
    }

    #[tokio::test]
    async fn create_ref_sends_full_ref_name() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/repos/o/r/git/refs"))
            .and(wiremock::matchers::body_json(
                json!({"ref":"refs/uploads/issue/1","sha":"sha1"}),
            ))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({})))
            .expect(1)
            .mount(&server)
            .await;
        client(&server)
            .create_ref("o", "r", "refs/uploads/issue/1", "sha1")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn update_ref_uses_no_prefix_path_and_force_true() {
        let server = MockServer::start().await;
        Mock::given(method("PATCH"))
            .and(path("/repos/o/r/git/refs/uploads/issue/1"))
            .and(wiremock::matchers::body_json(
                json!({"sha":"sha2","force":true}),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
            .expect(1)
            .mount(&server)
            .await;
        client(&server)
            .update_ref("o", "r", "uploads/issue/1", "sha2", true)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn list_refs_parses_array() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/o/r/git/matching-refs/uploads/issue"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([
                {"ref":"refs/uploads/issue/1","object":{"sha":"s1","type":"commit","url":"u"}}
            ])))
            .expect(1)
            .mount(&server)
            .await;
        let refs = client(&server)
            .list_refs("o", "r", "uploads/issue")
            .await
            .unwrap();
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].ref_name, "refs/uploads/issue/1");
        assert_eq!(refs[0].object_sha, "s1");
    }

    #[tokio::test]
    async fn list_refs_returns_empty_vec_on_empty_array() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/o/r/git/matching-refs/uploads/issue"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .expect(1)
            .mount(&server)
            .await;
        let refs = client(&server)
            .list_refs("o", "r", "uploads/issue")
            .await
            .unwrap();
        assert!(refs.is_empty());
    }

    #[tokio::test]
    async fn delete_ref_accepts_204() {
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path("/repos/o/r/git/refs/uploads/issue/1"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;
        client(&server)
            .delete_ref("o", "r", "uploads/issue/1")
            .await
            .unwrap();
    }

    // Error mapping tests

    #[tokio::test]
    async fn error_401_maps_to_auth_failure() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/repos/o/r/git/blobs"))
            .respond_with(ResponseTemplate::new(401).set_body_json(json!({"message":"Bad creds"})))
            .mount(&server)
            .await;
        let err = client(&server)
            .create_blob("o", "r", b"x")
            .await
            .unwrap_err();
        assert!(matches!(err, Error::AuthFailure));
    }

    #[tokio::test]
    async fn error_403_with_rate_limit_header_zero_maps_to_rate_limited() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/repos/o/r/git/blobs"))
            .respond_with(
                ResponseTemplate::new(403)
                    .append_header("x-ratelimit-remaining", "0")
                    .set_body_json(json!({"message":"Forbidden"})),
            )
            .mount(&server)
            .await;
        let err = client(&server)
            .create_blob("o", "r", b"x")
            .await
            .unwrap_err();
        assert!(matches!(err, Error::RateLimited { .. }));
    }

    #[tokio::test]
    async fn error_403_without_rate_limit_header_maps_to_api_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/repos/o/r/git/blobs"))
            .respond_with(ResponseTemplate::new(403).set_body_json(json!({"message":"Forbidden"})))
            .mount(&server)
            .await;
        let err = client(&server)
            .create_blob("o", "r", b"x")
            .await
            .unwrap_err();
        assert!(matches!(err, Error::ApiError { status: 403, .. }));
    }

    #[tokio::test]
    async fn error_404_from_non_get_ref_maps_to_api_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/repos/o/r/git/blobs"))
            .respond_with(ResponseTemplate::new(404).set_body_json(json!({"message":"Not Found"})))
            .mount(&server)
            .await;
        let err = client(&server)
            .create_blob("o", "r", b"x")
            .await
            .unwrap_err();
        assert!(matches!(err, Error::ApiError { status: 404, .. }));
    }

    #[tokio::test]
    async fn error_409_maps_to_empty_repo() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/repos/o/r/git/blobs"))
            .respond_with(ResponseTemplate::new(409).set_body_json(json!({"message":"conflict"})))
            .mount(&server)
            .await;
        let err = client(&server)
            .create_blob("o", "r", b"x")
            .await
            .unwrap_err();
        assert!(matches!(err, Error::EmptyRepo));
    }

    #[tokio::test]
    async fn error_422_maps_to_api_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/repos/o/r/git/blobs"))
            .respond_with(
                ResponseTemplate::new(422).set_body_json(json!({"message":"Unprocessable"})),
            )
            .mount(&server)
            .await;
        let err = client(&server)
            .create_blob("o", "r", b"x")
            .await
            .unwrap_err();
        assert!(matches!(err, Error::ApiError { status: 422, .. }));
    }

    #[tokio::test]
    async fn error_429_with_retry_after_header() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/repos/o/r/git/blobs"))
            .respond_with(
                ResponseTemplate::new(429)
                    .append_header("retry-after", "30")
                    .set_body_json(json!({"message":"too many requests"})),
            )
            .mount(&server)
            .await;
        let err = client(&server)
            .create_blob("o", "r", b"x")
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            Error::RateLimited {
                retry_after: Some(30)
            }
        ));
    }

    #[tokio::test]
    async fn user_agent_header_sent_on_every_request() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/o/r/git/matching-refs/uploads"))
            .and(header("user-agent", "gh-agent-screenshot"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .expect(1)
            .mount(&server)
            .await;
        client(&server)
            .list_refs("o", "r", "uploads")
            .await
            .unwrap();
    }

    // ── Issues/PRs REST API methods ────────────────────────────────────────────

    #[tokio::test]
    async fn create_comment_posts_to_issues_endpoint_and_returns_id() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/repos/o/r/issues/42/comments"))
            .and(wiremock::matchers::body_json(json!({"body": "hello"})))
            .respond_with(
                ResponseTemplate::new(201).set_body_json(json!({"id": 12345, "body": "hello"})),
            )
            .expect(1)
            .mount(&server)
            .await;
        let id = client(&server)
            .create_comment("o", "r", 42, "hello")
            .await
            .unwrap();
        assert_eq!(id, 12345);
    }

    #[tokio::test]
    async fn update_comment_patches_comment_endpoint() {
        let server = MockServer::start().await;
        Mock::given(method("PATCH"))
            .and(path("/repos/o/r/issues/comments/99"))
            .and(wiremock::matchers::body_json(json!({"body": "new"})))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"id": 99, "body": "new"})),
            )
            .expect(1)
            .mount(&server)
            .await;
        client(&server)
            .update_comment("o", "r", 99, "new")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn patch_issue_body_patches_issue_endpoint() {
        let server = MockServer::start().await;
        Mock::given(method("PATCH"))
            .and(path("/repos/o/r/issues/42"))
            .and(wiremock::matchers::body_json(json!({"body": "text"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"number": 42})))
            .expect(1)
            .mount(&server)
            .await;
        client(&server)
            .patch_issue_body("o", "r", 42, "text")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn patch_pr_body_patches_pulls_endpoint_not_issues() {
        let server = MockServer::start().await;
        // Asserts the URL path is /pulls/, NOT /issues/ — PR bodies live on a
        // different endpoint than issue/PR comments, which both use /issues/.
        Mock::given(method("PATCH"))
            .and(path("/repos/o/r/pulls/7"))
            .and(wiremock::matchers::body_json(json!({"body": "text"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"number": 7})))
            .expect(1)
            .mount(&server)
            .await;
        client(&server)
            .patch_pr_body("o", "r", 7, "text")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn get_commit_date_parses_committer_date() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/o/r/git/commits/abc123"))
            .and(header("authorization", "Bearer test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "sha": "abc123",
                "committer": {"date": "2024-01-15T12:34:56Z", "name": "x", "email": "y"},
                "author": {"date": "2024-01-15T12:34:56Z", "name": "x", "email": "y"},
                "message": "m",
                "tree": {"sha": "t", "url": "u"},
                "parents": []
            })))
            .expect(1)
            .mount(&server)
            .await;
        let dt = client(&server)
            .get_commit_date("o", "r", "abc123")
            .await
            .unwrap();
        let expected = Utc.with_ymd_and_hms(2024, 1, 15, 12, 34, 56).unwrap();
        assert_eq!(dt, expected);
    }

    #[tokio::test]
    async fn create_comment_404_returns_api_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/repos/o/r/issues/99/comments"))
            .respond_with(ResponseTemplate::new(404).set_body_json(json!({"message": "Not Found"})))
            .mount(&server)
            .await;
        let err = client(&server)
            .create_comment("o", "r", 99, "body")
            .await
            .unwrap_err();
        assert!(matches!(err, Error::ApiError { status: 404, .. }));
    }

    // ── append_body / append_* ──────────────────────────────────────────────

    #[test]
    fn append_body_empty_existing_returns_addition_only() {
        assert_eq!(append_body("", "new"), "new");
        assert_eq!(append_body("   \n  ", "new"), "new");
    }

    #[test]
    fn append_body_non_empty_joins_with_blank_line() {
        assert_eq!(append_body("old", "new"), "old\n\nnew");
        assert_eq!(append_body("old\n", "new"), "old\n\nnew");
    }

    #[tokio::test]
    async fn get_issue_body_returns_empty_string_on_null_body() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/o/r/issues/42"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"number": 42, "body": null})),
            )
            .expect(1)
            .mount(&server)
            .await;
        let body = client(&server).get_issue_body("o", "r", 42).await.unwrap();
        assert_eq!(body, "");
    }

    #[tokio::test]
    async fn get_pr_body_returns_empty_string_on_null_body() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/o/r/pulls/7"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"number": 7, "body": null})),
            )
            .expect(1)
            .mount(&server)
            .await;
        let body = client(&server).get_pr_body("o", "r", 7).await.unwrap();
        assert_eq!(body, "");
    }

    #[tokio::test]
    async fn get_comment_body_returns_empty_string_on_null_body() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/o/r/issues/comments/99"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": 99, "body": null})))
            .expect(1)
            .mount(&server)
            .await;
        let body = client(&server)
            .get_comment_body("o", "r", 99)
            .await
            .unwrap();
        assert_eq!(body, "");
    }

    #[tokio::test]
    async fn append_issue_body_gets_then_patches_joined_body() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/o/r/issues/42"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"number": 42, "body": "old"})),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("PATCH"))
            .and(path("/repos/o/r/issues/42"))
            .and(wiremock::matchers::body_json(json!({"body": "old\n\nnew"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"number": 42})))
            .expect(1)
            .mount(&server)
            .await;
        client(&server)
            .append_issue_body("o", "r", 42, "new")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn append_pr_body_gets_then_patches_joined_body() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/o/r/pulls/7"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"number": 7, "body": "old"})),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("PATCH"))
            .and(path("/repos/o/r/pulls/7"))
            .and(wiremock::matchers::body_json(json!({"body": "old\n\nnew"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"number": 7})))
            .expect(1)
            .mount(&server)
            .await;
        client(&server)
            .append_pr_body("o", "r", 7, "new")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn append_comment_gets_then_patches_joined_body() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/o/r/issues/comments/99"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"id": 99, "body": "old"})),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("PATCH"))
            .and(path("/repos/o/r/issues/comments/99"))
            .and(wiremock::matchers::body_json(json!({"body": "old\n\nnew"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": 99})))
            .expect(1)
            .mount(&server)
            .await;
        client(&server)
            .append_comment("o", "r", 99, "new")
            .await
            .unwrap();
    }
}
