---
name: gh-agent-screenshot
description: Use when asked to attach a screenshot or image to a GitHub issue or PR — uploads files inline via the Git Data API, works in private repos with a token only (no browser session). Triggers on "attach a screenshot to a GitHub issue/PR", "upload an image to a GitHub comment", "embed an image inline in a private GitHub repo", or "post a screenshot to a PR/issue".
license: MIT
metadata:
  author: mgiovani
  version: "0.1.0"
---

## What It Is

`gh agent-screenshot` is a `gh` CLI extension that uploads existing image files into GitHub issues and pull requests via the Git Data API, embedding them as inline image markdown that renders in the GitHub web UI — including private repositories — using only a GitHub token. No browser session, no third-party hosting, no S3.

Each upload creates a blob, assembles a tree and commit, and pushes to a `refs/uploads/<id>` branch. The resulting markdown embed uses a SHA-pinned `?raw=true` URL under `raw.githubusercontent.com`; the image renders inline for any collaborator authenticated to the repo via the session cookie. Old embed URLs are preserved on repeat uploads because each commit chains the prior commit as parent.

## Prerequisites

1. Install the extension:
   ```sh
   gh extension install mgiovani/gh-agent-screenshot
   ```
2. Authenticate `gh` (the extension inherits the token chain):
   ```sh
   gh auth login
   ```

## Usage — Upload

Upload one or more image files to a GitHub issue or PR. Exactly one of `--issue` / `--pr` is required. `--repo owner/name` is always required.

### Write modes

#### `--print-only` (default — no GitHub write)
Prints markdown image links to stdout without posting anything to GitHub.
```sh
gh agent-screenshot upload screenshot.png --repo owner/name --issue 42 --print-only
# or simply (--print-only is the default when no write mode is given)
gh agent-screenshot upload screenshot.png --repo owner/name --issue 42
```

#### `--new-comment`
Posts the images as a new comment on the issue or PR.
```sh
gh agent-screenshot upload before.png after.png --repo owner/name --pr 7 --new-comment
```

#### `--update-comment <id>`
Appends the images to an existing comment identified by its comment ID.
```sh
gh agent-screenshot upload diagram.png --repo owner/name --issue 10 --update-comment 1234567
```

#### `--edit-body`
Embeds the images in the issue or PR description (body) itself.
```sh
gh agent-screenshot upload arch.png --repo owner/name --pr 42 --edit-body
```

> `--issue` and `--pr` are mutually exclusive; exactly one is required.

## Usage — Prune

Remove stale upload branches (`refs/uploads/…`) that are no longer needed.

`--dry-run` and `--confirm` are mutually exclusive.

```sh
# Preview branches that would be deleted (no writes)
gh agent-screenshot prune --repo owner/name --dry-run

# Delete branches older than the default threshold (90 days)
gh agent-screenshot prune --repo owner/name --confirm

# Delete branches older than a custom threshold
gh agent-screenshot prune --repo owner/name --confirm --older-than-days 30
```

`--older-than-days` defaults to `90`.

## How It Works

Each upload pushes to a throwaway `refs/uploads/<id>` branch via the Git Data API (blob → tree → commit → ref update). The comment embeds a SHA-pinned `?raw=true` URL. Because the URL is same-origin (`raw.githubusercontent.com`) and the repository may be private, the browser authenticates the image request via the logged-in GitHub session cookie — no token is exposed in the markdown and the image renders inline for any collaborator with repo access.
