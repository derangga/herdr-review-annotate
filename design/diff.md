# Diff

Part of the [herdr-review design](../DESIGN.md).

## Commands

Every `git` call runs with `GIT_OPTIONAL_LOCKS=0` in its environment, so the pane never takes
`index.lock` while the agent runs its own `git` commands.

Common flags, written as `GIT` below:
`git -C <root> -c core.quotePath=false diff --no-color --no-ext-diff --no-textconv --src-prefix=a/ --dst-prefix=b/ --submodule=short -M -U3`
and `--` after the revision. `--submodule=short` is there because `diff.submodule = log` in a user's config
would otherwise replace the two commit ids with a log.

| Spec | Command |
|---|---|
| Working tree | `GIT HEAD`, then `git ls-files --others --exclude-standard -z` |
| Branch | `GIT $(git merge-base <base> HEAD)`, then the same `ls-files` |

Default base: `origin/HEAD` if it resolves, else `main`, else `master` (`diff::default_base`, `NoBase` when
none does). A base that starts with `-` is treated as missing and never reaches `git`. `open --base <ref>` overrides
it and the choice is saved in `meta.json`.

## Cases the pipeline must handle

| Case | Behaviour |
|---|---|
| Not a git repository | `open` shows a Herdr notification and exits non-zero |
| No commits (`HEAD` missing) | Diff against the empty tree, `git hash-object -t tree /dev/null` |
| Base missing | Message in the pane, working-tree spec offered |
| Empty diff | Message in the pane. Comments not in the diff are still listed |
| Untracked file | Stat first. Over 1 MiB or containing a NUL byte: listed, not rendered. Otherwise rendered as all-added lines. A symlink shows its target as one line. Anything that is not a file or a link (a nested repository) and any read error: listed, not rendered. The untracked files count against the 3 MiB cap too |
| Binary file | Listed with a "binary" row. File comments allowed |
| Total patch over 3 MiB | Files past the cap are listed and collapsed |
| Rename with edits | `old_path` from `rename from`. An old-side comment cites `old_path` (zeron's `cite_path`) |
| Submodule | One row with the two commit ids |
| CRLF | `\r` stripped for display and for anchor comparison |
| Deleted then recreated | Whatever `git` reports. No special handling |

Paths come from the `---`, `+++` and `rename` lines. A section with none of them (a binary file, a mode
change alone) has no other source, so the parser reads its `diff --git a/P b/P` line, and only when
both halves are the same path. A section it cannot read at all is `Unparsed`, listed under that path or
under a placeholder name.

## Anchor matching (ADR 0008)

For each root comment whose `spec` equals the current spec:

1. Same path, side and line, and the line text is equal: matched.
2. Else the nearest row in that file's hunks, on that side, with equal text: matched at the new line.
3. Else, if the file is in the diff: outdated, drawn at the stored line or the closest hunk.
4. Else: listed in "comments not in this diff".

Step 1 and step 2 are one search: the row on that side with equal text nearest the stored line, the earlier
one on a tie. The stored line itself is distance 0. A file is found by `path`, or by `old_path` when an agent
cited the old name. A file comment is matched whenever its file is in the diff. `Outdated.near` is the row on
that side nearest the stored line, in any hunk.

Comments written against the other spec go to the same list. Ranges match on their first line.

## Reload

The diff reloads when the store changes (the agent replying is the moment its fix landed), when the
user presses `R`, and when the pane regains focus. It never reloads while the editor is open.
