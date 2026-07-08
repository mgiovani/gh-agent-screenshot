<p align="center">
  <img src="docs/logo.png" width="420" alt="gh-agent-screenshot">
</p>

<p align="center">
  <a href="https://cli.github.com/manual/gh_extension"><img alt="gh extension" src="https://img.shields.io/badge/gh-extension-blue?logo=github"></a>
  <a href="LICENSE"><img alt="License: MIT" src="https://img.shields.io/badge/license-MIT-green.svg"></a>
  <a href="#supported-platforms"><img alt="Platforms" src="https://img.shields.io/badge/platforms-macOS%20%7C%20Linux%20%7C%20Windows-lightgrey"></a>
  <a href="Cargo.toml"><img alt="Built with Rust" src="https://img.shields.io/badge/built%20with-Rust-orange?logo=rust"></a>
  <a href="https://skills.sh/mgiovani/gh-agent-screenshot"><img alt="skills.sh" src="https://img.shields.io/badge/skills.sh-mgiovani%2Fgh--agent--screenshot-blueviolet"></a>
</p>

---

![Before and after: an agent describing a screenshot in words, then showing it](docs/hero.png)

Your coding agent writes code, opens pull requests, and fixes bugs on its own. It still cannot do the one thing you do by reflex: drop a screenshot into a comment. GitHub only accepts images dragged into its web editor, and there is no public API for it. gh-agent-screenshot closes that gap. One command uploads an image to any issue or PR and embeds it inline, stored in your own repo through GitHub's Git Data API. No Imgur, no S3, no upload token to leak.

## Install

```sh
gh extension install mgiovani/gh-agent-screenshot

# Optional: install the agent skill so AI coding agents know how to use it
npx skills add mgiovani/gh-agent-screenshot
```

The extension auto-selects the matching prebuilt binary for your host OS and architecture from the latest GitHub Release.

## Supported Platforms

| Asset name                          | OS      | Arch  |
|-------------------------------------|---------|-------|
| `gh-agent-screenshot-darwin-amd64`        | macOS   | x86_64 |
| `gh-agent-screenshot-darwin-arm64`        | macOS   | Apple Silicon |
| `gh-agent-screenshot-linux-amd64`         | Linux   | x86_64 |
| `gh-agent-screenshot-linux-arm64`         | Linux   | aarch64 |
| `gh-agent-screenshot-windows-amd64.exe`   | Windows | x86_64 |

## Usage

### Upload images

```sh
# Append images to the issue/PR body (default, no flag needed)
gh agent-screenshot upload a.png b.png --repo owner/name --pr 42

# Print markdown image links only, no GitHub write at all
gh agent-screenshot upload a.png b.png --repo owner/name --issue 1 --print-only

# Post images as a new comment
gh agent-screenshot upload a.png b.png --repo owner/name --issue 1 --new-comment

# Append images to an existing comment
gh agent-screenshot upload a.png b.png --repo owner/name --issue 1 --update-comment <id>

# Explicitly append to the issue/PR body (same as the default)
gh agent-screenshot upload a.png b.png --repo owner/name --pr 42 --edit-body

# Replace the body/comment instead of appending to it
gh agent-screenshot upload a.png b.png --repo owner/name --pr 42 --overwrite
```

`--issue` and `--pr` are mutually exclusive; one is required. The default write mode (no flag, or `--edit-body`) **appends** to the issue/PR body instead of replacing it. `--update-comment` also appends. Pass `--overwrite` to replace the body/comment instead — it's rejected with `--print-only` or `--new-comment`, which don't touch existing content. Appending reads the current body then writes the joined result, so two uploads racing against the same issue/PR/comment at the same time can still clobber each other; it isn't a concurrency-safe merge.

### Prune stale upload branches

```sh
# Preview which branches would be deleted (no writes)
gh agent-screenshot prune --repo owner/name --dry-run

# Delete branches older than the default threshold (90 days)
gh agent-screenshot prune --repo owner/name --confirm

# Delete branches older than a custom threshold
gh agent-screenshot prune --repo owner/name --confirm --older-than-days 30
```

`--dry-run` and `--confirm` are mutually exclusive. `--older-than-days` defaults to `90`.

## How It Works

Each upload creates a blob via the Git Data API, assembles a tree and commit, then pushes to a throwaway `refs/uploads/<id>` branch. The resulting comment embeds a `?raw=true` SHA-pinned URL that resolves directly to the blob content. Because the URL is under `raw.githubusercontent.com` and the repository is private, browsers authenticate the request via the logged-in GitHub session cookie: no token is exposed in the markdown, and the image renders inline for any collaborator who has repo access.

## Note on Private Repo Rendering

The private-repo inline render (image visible in a browser without a raw token) is verified by a logged-in browser session via cookie auth. CI cannot prove it automatically.

## Agent Skill

An agent skill is published so AI coding agents can learn how to use this extension. Install it with `npx skills add mgiovani/gh-agent-screenshot` (see [Install](#install)) or browse it on [skills.sh](https://skills.sh/mgiovani/gh-agent-screenshot).

The skill teaches agents all write modes (default body append, `--print-only`, `--new-comment`, `--update-comment`, `--edit-body`, `--overwrite`) and the `prune` subcommand. See [`skills/gh-agent-screenshot/SKILL.md`](skills/gh-agent-screenshot/SKILL.md).

## Credits

Licensed under the [MIT License](LICENSE).
