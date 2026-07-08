use gh_agent_screenshot::auth;
use gh_agent_screenshot::cli::{Cli, Command, PruneArgs, WriteMode};
use gh_agent_screenshot::error::{Error, Result};
use gh_agent_screenshot::git_data_api::GitHubClient;
use gh_agent_screenshot::prune::{run_prune, PruneMode};
use gh_agent_screenshot::upload;

use clap::Parser;

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    match run().await {
        Ok(()) => {}
        Err(e) => {
            eprintln!("gh-agent-screenshot: {}", e);
            std::process::exit(1);
        }
    }
}

async fn run() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Upload(a) => upload_cmd(a).await,
        Command::Prune(a) => prune_cmd(a).await,
    }
}

async fn upload_cmd(args: gh_agent_screenshot::cli::UploadArgs) -> Result<()> {
    args.validate_overwrite()?;

    for path in &args.files {
        if !path.is_file() {
            return Err(Error::FileNotFound {
                path: path.display().to_string(),
            });
        }
    }

    let token = auth::get_token()?;
    let (owner, repo) = args.split_repo()?;
    let client = GitHubClient::new(token);
    let write_mode = args.write_mode();

    let urls = upload::upload_batch(&client, &owner, &repo, &args.files, args.target()).await?;
    let markdown = upload::compose_markdown(&urls);

    match write_mode {
        WriteMode::PrintOnly => println!("{}", markdown),
        WriteMode::NewComment => {
            let id = client
                .create_comment(&owner, &repo, args.target().number(), &markdown)
                .await?;
            println!("{}", id);
        }
        WriteMode::UpdateComment(comment_id) => {
            if args.overwrite {
                client
                    .update_comment(&owner, &repo, comment_id, &markdown)
                    .await?;
            } else {
                client
                    .append_comment(&owner, &repo, comment_id, &markdown)
                    .await?;
            }
        }
        WriteMode::EditBody => match args.target() {
            upload::Target::Issue(n) => {
                if args.overwrite {
                    client.patch_issue_body(&owner, &repo, n, &markdown).await?;
                } else {
                    client
                        .append_issue_body(&owner, &repo, n, &markdown)
                        .await?;
                }
            }
            upload::Target::Pr(n) => {
                if args.overwrite {
                    client.patch_pr_body(&owner, &repo, n, &markdown).await?;
                } else {
                    client.append_pr_body(&owner, &repo, n, &markdown).await?;
                }
            }
        },
    }
    Ok(())
}

async fn prune_cmd(args: PruneArgs) -> Result<()> {
    if args.dry_run && args.confirm {
        return Err(Error::ApiError {
            status: 0,
            message: "--dry-run and --confirm cannot be used together; choose one.".into(),
        });
    }
    if !args.dry_run && !args.confirm {
        return Err(Error::ApiError {
            status: 0,
            message: "prune requires either --dry-run (list stale refs) or --confirm (delete stale refs).".into(),
        });
    }

    let (owner, repo) = args.split_repo()?;
    let token = auth::get_token()?;
    let client = GitHubClient::new(token);

    let mode = if args.dry_run {
        PruneMode::DryRun
    } else {
        PruneMode::Confirm
    };
    let report = run_prune(&client, &owner, &repo, args.older_than_days, mode).await?;

    for line in &report.stale_lines {
        println!("{}", line);
    }
    if !report.summary.is_empty() {
        println!("{}", report.summary);
    }
    Ok(())
}
