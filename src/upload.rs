use std::path::{Path, PathBuf};

use crate::error::Result;
use crate::git_data_api::{GitHubClient, TreeEntry};

pub enum Target {
    Issue(u32),
    Pr(u32),
}

impl Target {
    pub fn ref_segment(&self) -> &'static str {
        match self {
            Target::Issue(_) => "issue",
            Target::Pr(_) => "pr",
        }
    }

    pub fn number(&self) -> u32 {
        match self {
            Target::Issue(n) => *n,
            Target::Pr(n) => *n,
        }
    }
}

pub struct EmbedUrl(pub String);

impl std::fmt::Display for EmbedUrl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

fn percent_encode_path_component(s: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(s.len());
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char);
            }
            _ => {
                out.push('%');
                out.push(HEX[(byte >> 4) as usize] as char);
                out.push(HEX[(byte & 0xf) as usize] as char);
            }
        }
    }
    out
}

pub fn embed_url(owner: &str, repo: &str, commit_sha: &str, file_name: &str) -> EmbedUrl {
    let encoded = percent_encode_path_component(file_name);
    EmbedUrl(format!(
        "https://github.com/{}/{}/blob/{}/{}?raw=true",
        owner, repo, commit_sha, encoded
    ))
}

/// Upload multiple files as one atomic commit (all blobs in parallel, one tree, one commit).
/// Returns embed URLs in the same order as `file_paths`.
pub async fn upload_batch(
    client: &GitHubClient,
    owner: &str,
    repo: &str,
    file_paths: &[PathBuf],
    target: Target,
) -> Result<Vec<EmbedUrl>> {
    let file_names: Vec<String> = file_paths
        .iter()
        .map(|p| {
            p.file_name()
                .expect("file_path must have a file name component")
                .to_string_lossy()
                .to_string()
        })
        .collect();

    // Read all files in parallel (non-blocking I/O)
    let all_bytes = futures::future::try_join_all(file_paths.iter().map(tokio::fs::read)).await?;

    let ref_no_prefix = format!("uploads/{}/{}", target.ref_segment(), target.number());
    let full_ref = format!("refs/{}", ref_no_prefix);

    let existing = client.get_ref(owner, repo, &ref_no_prefix).await?;

    let base_tree: Option<String> = if let Some(ref r) = existing {
        Some(
            client
                .get_commit_tree_sha(owner, repo, &r.object_sha)
                .await?,
        )
    } else {
        None
    };

    // Upload all blobs in parallel, preserving original index for result ordering
    let blob_futs = all_bytes.iter().enumerate().map(|(i, bytes)| {
        let bytes = bytes.clone();
        async move {
            let sha = client.create_blob(owner, repo, &bytes).await?;
            Ok::<(usize, String), crate::error::Error>((i, sha))
        }
    });
    let indexed_blobs = futures::future::try_join_all(blob_futs).await?;

    // Sort tree entries by path so the tree API body is deterministic
    let mut tree_indexed: Vec<(usize, String, String)> = indexed_blobs
        .into_iter()
        .map(|(i, sha)| (i, file_names[i].clone(), sha))
        .collect();
    tree_indexed.sort_by(|a, b| a.1.cmp(&b.1));

    let tree_entries: Vec<TreeEntry> = tree_indexed
        .into_iter()
        .map(|(_, name, sha)| TreeEntry { path: name, sha })
        .collect();

    let tree_sha = client
        .create_tree(owner, repo, base_tree.as_deref(), &tree_entries)
        .await?;

    let parents: Vec<String> = existing
        .as_ref()
        .map(|r| vec![r.object_sha.clone()])
        .unwrap_or_default();

    // Single-file message matches legacy upload_single output; multi-file uses new format
    let commit_msg = if file_names.len() == 1 {
        format!("upload {}", file_names[0])
    } else {
        format!(
            "upload {} file(s): {}",
            file_names.len(),
            file_names.join(", ")
        )
    };

    let commit_sha = client
        .create_commit(owner, repo, &commit_msg, &tree_sha, &parents)
        .await?;

    if existing.is_none() {
        client
            .create_ref(owner, repo, &full_ref, &commit_sha)
            .await?;
    } else {
        client
            .update_ref(owner, repo, &ref_no_prefix, &commit_sha, true)
            .await?;
    }

    // Return URLs in input order (file_names preserves original order)
    Ok(file_names
        .iter()
        .map(|name| embed_url(owner, repo, &commit_sha, name))
        .collect())
}

/// Thin wrapper: upload a single file. Delegates to `upload_batch`.
pub async fn upload_single(
    client: &GitHubClient,
    owner: &str,
    repo: &str,
    file_path: &Path,
    target: Target,
) -> Result<EmbedUrl> {
    let mut urls = upload_batch(client, owner, repo, &[file_path.to_path_buf()], target).await?;
    Ok(urls.remove(0))
}

/// Render a slice of embed URLs as one `![](<url>)` per line joined by `\n`.
pub fn compose_markdown(urls: &[EmbedUrl]) -> String {
    urls.iter()
        .map(|u| format!("![]({})", u))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git_data_api::GitHubClient;
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn client(server: &MockServer) -> GitHubClient {
        GitHubClient::with_base_url("test-token".into(), server.uri())
    }

    // ── embed_url tests ────────────────────────────────────────────────────────

    #[test]
    fn embed_url_basic_format() {
        let sha = "a".repeat(40);
        let url = embed_url("o", "r", &sha, "img.png");
        assert_eq!(
            url.to_string(),
            format!("https://github.com/o/r/blob/{}/img.png?raw=true", sha)
        );
    }

    #[test]
    fn embed_url_encodes_spaces() {
        let sha = "a".repeat(40);
        let url = embed_url("o", "r", &sha, "foo bar.png");
        assert_eq!(
            url.to_string(),
            format!("https://github.com/o/r/blob/{}/foo%20bar.png?raw=true", sha)
        );
    }

    #[test]
    fn embed_url_encodes_hash_and_question() {
        let sha = "b".repeat(40);
        let url = embed_url("o", "r", &sha, "file#1?.png");
        assert_eq!(
            url.to_string(),
            format!(
                "https://github.com/o/r/blob/{}/file%231%3F.png?raw=true",
                sha
            )
        );
    }

    #[test]
    fn embed_url_encodes_non_ascii() {
        let sha = "c".repeat(40);
        // é is UTF-8: 0xC3 0xA9
        let url = embed_url("o", "r", &sha, "café.png");
        let s = url.to_string();
        assert!(s.contains("%C3%A9"), "expected %C3%A9 for é, got: {}", s);
        assert!(s.starts_with("https://github.com/o/r/blob/"));
        assert!(s.ends_with("?raw=true"));
    }

    // ── upload_single flows ────────────────────────────────────────────────────

    #[tokio::test]
    async fn upload_single_first_upload_no_existing_ref() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/repos/o/r/git/ref/uploads/issue/42"))
            .respond_with(ResponseTemplate::new(404).set_body_json(json!({"message": "Not Found"})))
            .expect(1)
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .and(path("/repos/o/r/git/blobs"))
            .respond_with(
                ResponseTemplate::new(201).set_body_json(json!({"sha": "blobsha1", "url": "x"})),
            )
            .expect(1)
            .mount(&server)
            .await;

        // body must omit base_tree
        Mock::given(method("POST"))
            .and(path("/repos/o/r/git/trees"))
            .and(wiremock::matchers::body_json(json!({
                "tree": [{"path": "test_first_upload.png", "mode": "100644", "type": "blob", "sha": "blobsha1"}]
            })))
            .respond_with(
                ResponseTemplate::new(201).set_body_json(json!({"sha": "treesha1"})),
            )
            .expect(1)
            .mount(&server)
            .await;

        // body must have parents: []
        Mock::given(method("POST"))
            .and(path("/repos/o/r/git/commits"))
            .and(wiremock::matchers::body_json(json!({
                "message": "upload test_first_upload.png",
                "tree": "treesha1",
                "parents": []
            })))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({"sha": "commitsha1"})))
            .expect(1)
            .mount(&server)
            .await;

        // body must use full ref name
        Mock::given(method("POST"))
            .and(path("/repos/o/r/git/refs"))
            .and(wiremock::matchers::body_json(json!({
                "ref": "refs/uploads/issue/42",
                "sha": "commitsha1"
            })))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({})))
            .expect(1)
            .mount(&server)
            .await;

        let tmp = std::env::temp_dir().join("test_first_upload.png");
        std::fs::write(&tmp, b"fake png data").unwrap();

        let result = upload_single(&client(&server), "o", "r", &tmp, Target::Issue(42))
            .await
            .unwrap();

        let _ = std::fs::remove_file(&tmp);

        assert_eq!(
            result.to_string(),
            "https://github.com/o/r/blob/commitsha1/test_first_upload.png?raw=true"
        );
    }

    #[tokio::test]
    async fn upload_single_repeat_upload_ref_exists() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/repos/o/r/git/ref/uploads/issue/42"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "ref": "refs/uploads/issue/42",
                "object": {"sha": "oldcommit", "type": "commit", "url": "x"}
            })))
            .expect(1)
            .mount(&server)
            .await;

        // get_commit_tree_sha fetches commit to retrieve tree sha
        Mock::given(method("GET"))
            .and(path("/repos/o/r/git/commits/oldcommit"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "sha": "oldcommit",
                "tree": {"sha": "oldtree", "url": "x"},
                "parents": []
            })))
            .expect(1)
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .and(path("/repos/o/r/git/blobs"))
            .respond_with(
                ResponseTemplate::new(201).set_body_json(json!({"sha": "blobsha2", "url": "x"})),
            )
            .expect(1)
            .mount(&server)
            .await;

        // body must include base_tree
        Mock::given(method("POST"))
            .and(path("/repos/o/r/git/trees"))
            .and(wiremock::matchers::body_json(json!({
                "base_tree": "oldtree",
                "tree": [{"path": "test_repeat_upload.png", "mode": "100644", "type": "blob", "sha": "blobsha2"}]
            })))
            .respond_with(
                ResponseTemplate::new(201).set_body_json(json!({"sha": "treesha2"})),
            )
            .expect(1)
            .mount(&server)
            .await;

        // body must have parents: ["oldcommit"]
        Mock::given(method("POST"))
            .and(path("/repos/o/r/git/commits"))
            .and(wiremock::matchers::body_json(json!({
                "message": "upload test_repeat_upload.png",
                "tree": "treesha2",
                "parents": ["oldcommit"]
            })))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({"sha": "commitsha2"})))
            .expect(1)
            .mount(&server)
            .await;

        // PATCH with force: true
        Mock::given(method("PATCH"))
            .and(path("/repos/o/r/git/refs/uploads/issue/42"))
            .and(wiremock::matchers::body_json(json!({
                "sha": "commitsha2",
                "force": true
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
            .expect(1)
            .mount(&server)
            .await;

        let tmp = std::env::temp_dir().join("test_repeat_upload.png");
        std::fs::write(&tmp, b"fake png data v2").unwrap();

        let result = upload_single(&client(&server), "o", "r", &tmp, Target::Issue(42))
            .await
            .unwrap();

        let _ = std::fs::remove_file(&tmp);

        assert_eq!(
            result.to_string(),
            "https://github.com/o/r/blob/commitsha2/test_repeat_upload.png?raw=true"
        );
    }

    #[tokio::test]
    async fn target_pr_variant_uses_singular_pr_in_ref_path() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/repos/o/r/git/ref/uploads/pr/7"))
            .respond_with(ResponseTemplate::new(404).set_body_json(json!({"message": "Not Found"})))
            .expect(1)
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .and(path("/repos/o/r/git/blobs"))
            .respond_with(
                ResponseTemplate::new(201).set_body_json(json!({"sha": "bs", "url": "x"})),
            )
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .and(path("/repos/o/r/git/trees"))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({"sha": "ts"})))
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .and(path("/repos/o/r/git/commits"))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({"sha": "cs"})))
            .mount(&server)
            .await;

        // singular "pr", not "prs"
        Mock::given(method("POST"))
            .and(path("/repos/o/r/git/refs"))
            .and(wiremock::matchers::body_json(json!({
                "ref": "refs/uploads/pr/7",
                "sha": "cs"
            })))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({})))
            .expect(1)
            .mount(&server)
            .await;

        let tmp = std::env::temp_dir().join("test_pr_upload.png");
        std::fs::write(&tmp, b"img").unwrap();

        let result = upload_single(&client(&server), "o", "r", &tmp, Target::Pr(7))
            .await
            .unwrap();

        let _ = std::fs::remove_file(&tmp);

        assert!(
            result.to_string().contains("/blob/cs/test_pr_upload.png"),
            "URL should contain commit sha and filename: {}",
            result
        );
    }

    // ── upload_batch flows ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn upload_batch_two_files_first_upload_no_base_tree() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/repos/o/r/git/ref/uploads/issue/1"))
            .respond_with(ResponseTemplate::new(404).set_body_json(json!({"message": "Not Found"})))
            .expect(1)
            .mount(&server)
            .await;

        // Both blobs respond with the same sha; sorted tree entries become deterministic
        Mock::given(method("POST"))
            .and(path("/repos/o/r/git/blobs"))
            .respond_with(
                ResponseTemplate::new(201).set_body_json(json!({"sha": "blobsha", "url": "x"})),
            )
            .expect(2)
            .mount(&server)
            .await;

        // Entries sorted by path: batch1_alpha before batch1_zulu; no base_tree
        Mock::given(method("POST"))
            .and(path("/repos/o/r/git/trees"))
            .and(wiremock::matchers::body_json(json!({
                "tree": [
                    {"path": "batch1_alpha.png", "mode": "100644", "type": "blob", "sha": "blobsha"},
                    {"path": "batch1_zulu.png",  "mode": "100644", "type": "blob", "sha": "blobsha"}
                ]
            })))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({"sha": "treesha"})))
            .expect(1)
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .and(path("/repos/o/r/git/commits"))
            .and(wiremock::matchers::body_json(json!({
                "message": "upload 2 file(s): batch1_alpha.png, batch1_zulu.png",
                "tree": "treesha",
                "parents": []
            })))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({"sha": "commitsha"})))
            .expect(1)
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .and(path("/repos/o/r/git/refs"))
            .and(wiremock::matchers::body_json(json!({
                "ref": "refs/uploads/issue/1",
                "sha": "commitsha"
            })))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({})))
            .expect(1)
            .mount(&server)
            .await;

        let tmp = std::env::temp_dir();
        let f1 = tmp.join("batch1_alpha.png");
        let f2 = tmp.join("batch1_zulu.png");
        std::fs::write(&f1, b"fake alpha").unwrap();
        std::fs::write(&f2, b"fake zulu").unwrap();

        let result = upload_batch(
            &client(&server),
            "o",
            "r",
            &[f1.clone(), f2.clone()],
            Target::Issue(1),
        )
        .await
        .unwrap();

        let _ = std::fs::remove_file(&f1);
        let _ = std::fs::remove_file(&f2);

        assert_eq!(result.len(), 2);
        assert!(
            result[0].to_string().contains("batch1_alpha.png"),
            "first URL must be for alpha file, got: {}",
            result[0]
        );
        assert!(
            result[1].to_string().contains("batch1_zulu.png"),
            "second URL must be for zulu file, got: {}",
            result[1]
        );
    }

    #[tokio::test]
    async fn upload_batch_two_files_repeat_upload_with_base_tree() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/repos/o/r/git/ref/uploads/issue/2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "ref": "refs/uploads/issue/2",
                "object": {"sha": "oldcommit", "type": "commit", "url": "x"}
            })))
            .expect(1)
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/repos/o/r/git/commits/oldcommit"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "sha": "oldcommit",
                "tree": {"sha": "oldtree", "url": "x"},
                "parents": []
            })))
            .expect(1)
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .and(path("/repos/o/r/git/blobs"))
            .respond_with(
                ResponseTemplate::new(201).set_body_json(json!({"sha": "blobsha", "url": "x"})),
            )
            .expect(2)
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .and(path("/repos/o/r/git/trees"))
            .and(wiremock::matchers::body_json(json!({
                "base_tree": "oldtree",
                "tree": [
                    {"path": "batch2_alpha.png", "mode": "100644", "type": "blob", "sha": "blobsha"},
                    {"path": "batch2_zulu.png",  "mode": "100644", "type": "blob", "sha": "blobsha"}
                ]
            })))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({"sha": "treesha2"})))
            .expect(1)
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .and(path("/repos/o/r/git/commits"))
            .and(wiremock::matchers::body_json(json!({
                "message": "upload 2 file(s): batch2_alpha.png, batch2_zulu.png",
                "tree": "treesha2",
                "parents": ["oldcommit"]
            })))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({"sha": "commitsha2"})))
            .expect(1)
            .mount(&server)
            .await;

        Mock::given(method("PATCH"))
            .and(path("/repos/o/r/git/refs/uploads/issue/2"))
            .and(wiremock::matchers::body_json(json!({
                "sha": "commitsha2",
                "force": true
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
            .expect(1)
            .mount(&server)
            .await;

        let tmp = std::env::temp_dir();
        let f1 = tmp.join("batch2_alpha.png");
        let f2 = tmp.join("batch2_zulu.png");
        std::fs::write(&f1, b"fake alpha v2").unwrap();
        std::fs::write(&f2, b"fake zulu v2").unwrap();

        let result = upload_batch(
            &client(&server),
            "o",
            "r",
            &[f1.clone(), f2.clone()],
            Target::Issue(2),
        )
        .await
        .unwrap();

        let _ = std::fs::remove_file(&f1);
        let _ = std::fs::remove_file(&f2);

        assert_eq!(result.len(), 2);
        assert!(result[0].to_string().contains("batch2_alpha.png"));
        assert!(result[1].to_string().contains("batch2_zulu.png"));
    }

    #[tokio::test]
    async fn upload_batch_preserves_input_order() {
        // Input order: c, a, b — tree must be sorted (a, b, c) but URLs returned in input order
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/repos/o/r/git/ref/uploads/issue/3"))
            .respond_with(ResponseTemplate::new(404).set_body_json(json!({"message": "Not Found"})))
            .expect(1)
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .and(path("/repos/o/r/git/blobs"))
            .respond_with(
                ResponseTemplate::new(201).set_body_json(json!({"sha": "blobsha", "url": "x"})),
            )
            .expect(3)
            .mount(&server)
            .await;

        // Tree entries sorted alphabetically: order_a, order_b, order_c
        Mock::given(method("POST"))
            .and(path("/repos/o/r/git/trees"))
            .and(wiremock::matchers::body_json(json!({
                "tree": [
                    {"path": "order_a.png", "mode": "100644", "type": "blob", "sha": "blobsha"},
                    {"path": "order_b.png", "mode": "100644", "type": "blob", "sha": "blobsha"},
                    {"path": "order_c.png", "mode": "100644", "type": "blob", "sha": "blobsha"}
                ]
            })))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({"sha": "treesha3"})))
            .expect(1)
            .mount(&server)
            .await;

        // Commit message lists files in input order: c, a, b
        Mock::given(method("POST"))
            .and(path("/repos/o/r/git/commits"))
            .and(wiremock::matchers::body_json(json!({
                "message": "upload 3 file(s): order_c.png, order_a.png, order_b.png",
                "tree": "treesha3",
                "parents": []
            })))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({"sha": "commitsha3"})))
            .expect(1)
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .and(path("/repos/o/r/git/refs"))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({})))
            .expect(1)
            .mount(&server)
            .await;

        let tmp = std::env::temp_dir();
        let fc = tmp.join("order_c.png");
        let fa = tmp.join("order_a.png");
        let fb = tmp.join("order_b.png");
        std::fs::write(&fc, b"data c").unwrap();
        std::fs::write(&fa, b"data a").unwrap();
        std::fs::write(&fb, b"data b").unwrap();

        // Pass in order: c, a, b
        let result = upload_batch(
            &client(&server),
            "o",
            "r",
            &[fc.clone(), fa.clone(), fb.clone()],
            Target::Issue(3),
        )
        .await
        .unwrap();

        let _ = std::fs::remove_file(&fc);
        let _ = std::fs::remove_file(&fa);
        let _ = std::fs::remove_file(&fb);

        assert_eq!(result.len(), 3);
        assert!(
            result[0].to_string().contains("order_c.png"),
            "result[0] must be c (input order), got: {}",
            result[0]
        );
        assert!(
            result[1].to_string().contains("order_a.png"),
            "result[1] must be a (input order), got: {}",
            result[1]
        );
        assert!(
            result[2].to_string().contains("order_b.png"),
            "result[2] must be b (input order), got: {}",
            result[2]
        );
    }

    #[tokio::test]
    async fn upload_batch_single_file_matches_upload_single_shape() {
        // Verifies 1-file batch uses exactly 1 blob, 1 tree, 1 commit, 1 ref op
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/repos/o/r/git/ref/uploads/issue/4"))
            .respond_with(ResponseTemplate::new(404).set_body_json(json!({"message": "Not Found"})))
            .expect(1)
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .and(path("/repos/o/r/git/blobs"))
            .respond_with(
                ResponseTemplate::new(201).set_body_json(json!({"sha": "bs", "url": "x"})),
            )
            .expect(1)
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .and(path("/repos/o/r/git/trees"))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({"sha": "ts"})))
            .expect(1)
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .and(path("/repos/o/r/git/commits"))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({"sha": "cs"})))
            .expect(1)
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .and(path("/repos/o/r/git/refs"))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({})))
            .expect(1)
            .mount(&server)
            .await;

        let tmp = std::env::temp_dir().join("batch_shape_single.png");
        std::fs::write(&tmp, b"single file data").unwrap();

        let result = upload_batch(
            &client(&server),
            "o",
            "r",
            std::slice::from_ref(&tmp),
            Target::Issue(4),
        )
        .await
        .unwrap();

        let _ = std::fs::remove_file(&tmp);

        assert_eq!(result.len(), 1);
        assert!(result[0].to_string().contains("batch_shape_single.png"));
    }

    // ── compose_markdown tests ─────────────────────────────────────────────────

    #[test]
    fn compose_markdown_joins_with_newline() {
        let sha = "a".repeat(40);
        let urls = vec![
            embed_url("o", "r", &sha, "c.png"),
            embed_url("o", "r", &sha, "a.png"),
            embed_url("o", "r", &sha, "b.png"),
        ];
        let md = compose_markdown(&urls);
        let lines: Vec<&str> = md.split('\n').collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].contains("c.png"));
        assert!(lines[1].contains("a.png"));
        assert!(lines[2].contains("b.png"));
        assert!(lines[0].starts_with("![](") && lines[0].ends_with(')'));
    }

    #[test]
    fn compose_markdown_single_url() {
        let sha = "b".repeat(40);
        let urls = vec![embed_url("o", "r", &sha, "img.png")];
        let md = compose_markdown(&urls);
        assert_eq!(
            md,
            format!("![](https://github.com/o/r/blob/{}/img.png?raw=true)", sha)
        );
        assert!(
            !md.ends_with('\n'),
            "single URL must not have trailing newline"
        );
    }
}
