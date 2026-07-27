---
name: gh-agent-screenshot
description: Use when asked to attach a screenshot or image to a GitHub issue or PR — uploads files inline via the Git Data API, works in private repos with a token only (no browser session). Triggers on "attach a screenshot to a GitHub issue/PR", "upload an image to a GitHub comment", "embed an image inline in a private GitHub repo", or "post a screenshot to a PR/issue".
license: MIT
metadata:
  author: mgiovani
  version: "0.2.1"
---

## What It Is

`gh agent-screenshot` is a `gh` CLI extension that uploads existing image files into GitHub issues and pull requests via the Git Data API, embedding them as inline image markdown that renders in the GitHub web UI — including private repositories — using only a GitHub token. No browser session, no third-party hosting, no S3.

Each upload creates a blob, assembles a tree and commit, and pushes to a `refs/uploads/<id>` branch. The resulting markdown embed uses a SHA-pinned `?raw=true` URL under `raw.githubusercontent.com`; the image renders inline for any collaborator authenticated to the repo via the session cookie. Old embed URLs are preserved on repeat uploads because each commit chains the prior commit as parent.

## Prerequisites

1. Install the extension, pinned to a released tag (never install unpinned — the
   binary inherits your `gh` token):
   ```sh
   gh extension install mgiovani/gh-agent-screenshot --pin v0.2.1
   ```
   Upgrading is deliberate: re-run the command with a newer tag.
2. Authenticate `gh` (the extension inherits the token chain):
   ```sh
   gh auth login
   ```

## Usage — Upload

Upload one or more image files to a GitHub issue or PR. Exactly one of `--issue` / `--pr` is required. `--repo owner/name` is always required.

**Safe by default: no write mode replaces existing content unless you pass `--overwrite`.** Appending is a read-then-write (GET the current body, join, PATCH), not an atomic merge — two uploads racing against the same issue/PR/comment at the same instant can still clobber each other.

### Write modes

#### Default (no flag) or `--edit-body`
Appends the images to the issue or PR description (body), after whatever text is already there. This is the default — no flag needed.
```sh
gh agent-screenshot upload arch.png --repo owner/name --pr 42
# equivalent, explicit form:
gh agent-screenshot upload arch.png --repo owner/name --pr 42 --edit-body
```

#### `--update-comment <id>`
Appends the images to an existing comment identified by its comment ID.
```sh
gh agent-screenshot upload diagram.png --repo owner/name --issue 10 --update-comment 1234567
```

#### `--new-comment`
Posts the images as a new comment on the issue or PR.
```sh
gh agent-screenshot upload before.png after.png --repo owner/name --pr 7 --new-comment
```

#### `--print-only`
Prints markdown image links to stdout without posting anything to GitHub.
```sh
gh agent-screenshot upload screenshot.png --repo owner/name --issue 42 --print-only
```

#### `--overwrite`
Modifier for the default body-append, `--edit-body`, or `--update-comment`: replaces the existing body/comment instead of appending to it. Rejected with `--print-only` or `--new-comment`, since neither touches existing content.
```sh
gh agent-screenshot upload arch.png --repo owner/name --pr 42 --overwrite
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
