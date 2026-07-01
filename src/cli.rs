use clap::{Args, Parser, Subcommand};

use crate::upload;

#[derive(Parser)]
#[command(
    name = "gh-agent-screenshot",
    about = "Upload images to GitHub via the Git Data API"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    Upload(UploadArgs),
    Prune(PruneArgs),
}

#[derive(Debug, PartialEq)]
pub enum WriteMode {
    PrintOnly,
    NewComment,
    UpdateComment(u64),
    EditBody,
}

#[derive(Args)]
#[group(required = false, multiple = false)]
pub struct WriteModeArgs {
    #[arg(long)]
    pub new_comment: bool,
    #[arg(long, value_name = "ID")]
    pub update_comment: Option<u64>,
    #[arg(long)]
    pub edit_body: bool,
    #[arg(long)]
    pub print_only: bool,
}

#[derive(Args)]
pub struct UploadArgs {
    pub files: Vec<std::path::PathBuf>,
    #[arg(long)]
    pub repo: String,
    #[command(flatten)]
    pub target: TargetArgs,
    #[command(flatten)]
    pub write_mode_args: WriteModeArgs,
}

#[derive(Args)]
#[group(required = true, multiple = false)]
pub struct TargetArgs {
    #[arg(long)]
    pub issue: Option<u32>,
    #[arg(long)]
    pub pr: Option<u32>,
}

#[derive(Args)]
pub struct PruneArgs {
    #[arg(long)]
    pub repo: String,
    #[arg(long)]
    pub dry_run: bool,
    #[arg(long)]
    pub confirm: bool,
    #[arg(long, default_value_t = 90)]
    pub older_than_days: u64,
}

pub fn split_repo(repo: &str) -> crate::error::Result<(String, String)> {
    let Some((owner, r)) = repo.split_once('/') else {
        return Err(crate::error::Error::ApiError {
            status: 0,
            message: format!(
                "invalid --repo '{}': expected format 'owner/repo' (e.g. 'myorg/myrepo')",
                repo
            ),
        });
    };
    Ok((owner.to_string(), r.to_string()))
}

impl UploadArgs {
    pub fn target(&self) -> upload::Target {
        if let Some(n) = self.target.issue {
            upload::Target::Issue(n)
        } else if let Some(n) = self.target.pr {
            upload::Target::Pr(n)
        } else {
            unreachable!("clap ArgGroup ensures one of --issue or --pr is set")
        }
    }

    pub fn write_mode(&self) -> WriteMode {
        let w = &self.write_mode_args;
        if w.new_comment {
            WriteMode::NewComment
        } else if let Some(id) = w.update_comment {
            WriteMode::UpdateComment(id)
        } else if w.edit_body {
            WriteMode::EditBody
        } else {
            WriteMode::PrintOnly
        }
    }

    pub fn split_repo(&self) -> crate::error::Result<(String, String)> {
        split_repo(&self.repo)
    }
}

impl PruneArgs {
    pub fn split_repo(&self) -> crate::error::Result<(String, String)> {
        split_repo(&self.repo)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn parse_upload(args: &[&str]) -> std::result::Result<UploadArgs, clap::Error> {
        let full_args: Vec<&str> = ["gh-agent-screenshot", "upload"]
            .iter()
            .chain(args.iter())
            .copied()
            .collect();
        match Cli::try_parse_from(full_args) {
            Ok(cli) => match cli.command {
                Command::Upload(a) => Ok(a),
                _ => panic!("expected upload subcommand"),
            },
            Err(e) => Err(e),
        }
    }

    #[test]
    fn write_mode_defaults_to_print_only() {
        let args = parse_upload(&["a.png", "--repo", "o/r", "--issue", "1"]).unwrap();
        assert_eq!(args.write_mode(), WriteMode::PrintOnly);
    }

    #[test]
    fn write_mode_new_comment() {
        let args =
            parse_upload(&["a.png", "--repo", "o/r", "--issue", "1", "--new-comment"]).unwrap();
        assert_eq!(args.write_mode(), WriteMode::NewComment);
    }

    #[test]
    fn write_mode_update_comment() {
        let args = parse_upload(&[
            "a.png",
            "--repo",
            "o/r",
            "--issue",
            "1",
            "--update-comment",
            "123",
        ])
        .unwrap();
        assert_eq!(args.write_mode(), WriteMode::UpdateComment(123));
    }

    #[test]
    fn write_mode_edit_body() {
        let args =
            parse_upload(&["a.png", "--repo", "o/r", "--issue", "1", "--edit-body"]).unwrap();
        assert_eq!(args.write_mode(), WriteMode::EditBody);
    }

    #[test]
    fn write_mode_explicit_print_only() {
        let args =
            parse_upload(&["a.png", "--repo", "o/r", "--issue", "1", "--print-only"]).unwrap();
        assert_eq!(args.write_mode(), WriteMode::PrintOnly);
    }

    #[test]
    fn write_mode_conflicting_flags_rejected() {
        let result = parse_upload(&[
            "a.png",
            "--repo",
            "o/r",
            "--issue",
            "1",
            "--new-comment",
            "--edit-body",
        ]);
        assert!(
            result.is_err(),
            "conflicting write mode flags must be rejected by clap"
        );
    }

    #[test]
    fn multiple_files_accepted() {
        let args =
            parse_upload(&["a.png", "b.png", "c.png", "--repo", "o/r", "--issue", "1"]).unwrap();
        assert_eq!(args.files.len(), 3);
    }
}
