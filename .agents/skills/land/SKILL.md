---
name: land
description: >-
  Use only when the user has explicitly requested landing the current project changes,
  such as by choosing Land Changes or invoking /land. For this repository, create a
  topic branch, commit the requested changes, push it to origin, and open a detailed
  pull request against main. Do not invoke this skill for review, preparation,
  validation, or skill-installation requests alone.
metadata:
  delta-action: land
---

# Land changes as a pull request

Carry out the explicit landing request in the current `bili-shadowreplay` checkout. The
requested landing outcome for this project is a published pull request containing the
current change; do not merge the pull request unless the user separately requests a
merge.

The request that invoked this skill already authorizes the workflow below. Do not ask
whether to create a branch, commit, push, or open the pull request again. Stop only for
a genuine blocker, an unsafe scope, an ambiguous conflict, or a failed required check.

## Project facts and constraints

- The repository is `Xinrea/bili-shadowreplay`.
- `origin` is the publication remote. The default and pull-request base branch is
  `main`.
- There is no repository pull-request template or documented CLA, signing requirement,
  mandatory reviewer, changelog entry, or issue reference requirement. Do not invent
  any of these requirements.
- Pull requests trigger the `Lint` and `Tests` workflows. The active repository ruleset
  currently prevents deletion and non-fast-forward updates to the default branch; it
  does not define required status checks. Treat the project `Lint` and `Tests` jobs,
  plus any status checks explicitly required by the current ruleset, as required for
  this workflow.
- Use non-interactive commands. Never print credentials, tokens, or complete
  environment dumps.

## Preconditions and scope safety

1. Confirm that this skill was invoked for an explicit landing request. If it was
   invoked for any other purpose, stop without changing Git state.
2. Resolve the repository root with `git rev-parse --show-toplevel` and work there.
   Confirm that `origin` exists and points to the intended `Xinrea/bili-shadowreplay`
   repository. Confirm the GitHub CLI is authenticated with `gh auth status`; do not
   expose its token output. Stop if authentication or the remote is unavailable.
3. Inspect `git status --short --branch`, the staged diff, the unstaged diff, and the
   untracked-file list. The expected landing scope is the current requested change. In
   the setup flow, the newly generated `.agents/skills/land/SKILL.md` is an explicitly
   requested file and may be the sole untracked path; preserve every other untracked
   path and stop if one is present. Refuse to proceed when the index already contains
   staged changes or when the scope cannot be distinguished from unrelated work. Do not
   reset, clean, stash, or overwrite those files. Stop if there is no requested diff to
   commit.
4. Fetch the target branch without changing the working tree:

   ```sh
   git fetch origin main
   ```

   Confirm that the current checkout is based on the current `origin/main`. If the
   checkout has diverged or the target moved in a way that makes the intended base
   unclear, stop rather than silently rewriting work. A clean, unambiguous update may
   be incorporated later using the conflict procedure below.
5. Run `git diff --check` over the requested changes. Do not continue while it reports
   whitespace errors.

## Validate before committing

Run the repository-defined checks that are available before staging and committing.
Record each command and its result for the pull-request body.

- The exact lint workflow command is `pre-commit run --all-files
  --show-diff-on-failure`, from `.github/workflows/lint.yml` lines 62-66. Run it when
  `pre-commit` is installed. If it is unavailable, record that local lint was not run;
  do not claim it passed or install packages into the user's environment without an
  explicit request.
- From `src-tauri`, run the two test commands used by the PR workflow, from
  `.github/workflows/test.yml` lines 32-46:

  ```sh
  cargo test --workspace --lib --exclude whisper-cpp-rs
  cargo test --package recorder --tests
  ```

  Preserve the workflow's `PKG_CONFIG_PATH` and `RUST_BACKTRACE` environment settings
  when they are needed by the host. If a required local command cannot run, record the
  reason and continue only if the corresponding remote check can provide the
  verification; never report an unrun check as passed.
- If a check changes files (for example, a formatter hook), inspect the new diff and
  include only changes that are clearly part of the requested fix. Re-run the affected
  checks after such changes.

If a local check fails because of the requested change, fix the change when the fix is
clear, then rerun the check. Do not bypass hooks or suppress failures. Stop for an
environment failure whose impact cannot be assessed.

## Create the branch and commit

1. Choose a short conventional branch name with a type prefix and a kebab-case
   description, derived from the actual change (for example,
   `fix/remove-duplicate-single-instance`). Check both local refs and `origin` for a
   collision. If the name exists, append `-2`, `-3`, and so on; never reuse or
   overwrite an existing branch.
2. Create the branch from the verified current checkout:

   ```sh
   git switch -c <branch>
   ```

3. Stage only the requested changes. Use `git add -u`, and when the setup flow is
   landing this skill itself, also stage the explicitly allowed
   `.agents/skills/land/SKILL.md` path. Inspect `git diff --cached --stat` and
   `git diff --cached --check`. Stop if the staged paths differ from the requested
   scope.
4. Create one non-interactive conventional commit with a concise subject that explains
   the behavior change. Use `GIT_EDITOR=true git commit -m "<subject>"`; do not amend an
   existing commit or rewrite history. Verify the commit SHA, subject, and changed
   paths afterward.
5. Confirm the worktree is clean and the new commit is based on the intended `main`
   tip. If a commit hook fails, fix the underlying issue and create a new commit only
   when necessary; never bypass the hook.

## Push and create the pull request

1. Publish only the newly created branch:

   ```sh
   git push --set-upstream origin <branch>
   ```

   Do not force-push. If the push is rejected because the remote branch appeared after
   the collision check, stop and choose a new branch rather than overwriting it.
2. Build a temporary pull-request body outside the repository and remove it after use.
   Do not commit the body file. The body must be detailed and based on the actual diff,
   with these sections:

   - `## Summary`: the user-visible problem and the resulting behavior.
   - `## Changes`: every affected file or logical change, without claiming unrelated
     work.
   - `## Validation`: each local command, its result, and any unavailable local check;
     link the eventual GitHub checks after they exist.
   - `## Behavior`: include at least one valid fenced `mermaid` diagram. For the
     single-instance fix, the diagram should show startup registering one
     focus-aware plugin and the second launch focusing the existing main window. Quote
     Mermaid labels containing punctuation and do not use HTML tags in the diagram.

   For example, an appropriate diagram shape is:

   ```mermaid
   flowchart TD
       A["Application starts"] --> B["Register one single-instance plugin"]
       B --> C["Second launch focuses the existing main window"]
       C --> D["Continue normal application initialization"]
   ```

   Ensure the body does not contain secrets, local absolute paths, or unsupported
   claims. Include the commit SHA and validation details only after verifying them.
3. Create the pull request non-interactively with the explicit repository, base, head,
   title, and body-file flags supported by `gh pr create`:

   ```sh
   gh pr create \
     --repo Xinrea/bili-shadowreplay \
     --base main \
     --head <branch> \
     --title "<title>" \
     --body-file <temporary-body-file>
   ```

   Capture the returned PR URL. Verify it with `gh pr view <url>` and confirm that the
   base is `main`, the head is the new branch, the title is correct, and the body
   contains the requested Mermaid diagram. If creation fails, report that the branch
   may be published but the requested PR was not created; do not claim success.

## Conflicts and recovery

The conflict preference for this project is to resolve conflicts automatically when the
intended result is clear, and to pause when it is ambiguous.

- If `origin/main` advances after branch creation or GitHub reports a merge conflict,
  fetch `origin/main` and merge it into the new topic branch with a normal merge. Keep
  the requested change and preserve unrelated upstream changes. Resolve only conflicts
  whose intended result is unambiguous from the surrounding code and the request.
- After resolving a clear conflict, run `git diff --check`, the affected local checks,
  and the full required test commands again; commit the merge or fix non-interactively
  and push normally.
- Stop and report the conflicting paths when the correct resolution is unclear, when a
  resolution would alter unrelated behavior, or when the branch contains commits not
  created by this workflow. Do not use `git checkout --ours`, `git checkout --theirs`,
  `git reset --hard`, force-push, or a history rewrite as a shortcut.

## Verify the published result

After the PR exists, verify all checks required by the current repository configuration.
Do not treat the PR's existence, a successful push, or a pending check as success.

1. Use `gh pr checks <url> --watch` (with a non-interactive interval if needed) and
   inspect the named workflow/job results. The project jobs are `Lint`/`pr-check` and
   `Tests`/`Run Tests`, as defined in `.github/workflows/lint.yml` lines 12-15 and
   `.github/workflows/test.yml` lines 9-12. Also inspect any checks explicitly required
   by the current GitHub ruleset.
2. A required check that is pending, failing, skipped unexpectedly, missing, or
   unverifiable is a blocker. If a failure is directly caused by the requested code and
   the repair is clear, add a corrective commit, push it normally, and wait for the new
   checks. Otherwise stop with the PR still open and report the blocker with its actual
   check URL.
3. When all required checks pass, verify the final PR head SHA still matches the
   verified commit (or the final corrective commit), the worktree is clean, and the PR
   remains open against `main`. Record the short commit SHA and actual check URLs.

A successful outcome means the requested commit is on the new branch, the branch is on
`origin`, the detailed PR with a Mermaid diagram exists against `main`, and all required
checks have passed. It does not mean that `main` was merged.

## Outcome reporting

When running in a subthread and `report_subthread_status` is available, report the
outcome to the parent only after verification:

- On success, use `status: "success"`, a short sentence-case title such as `PR created`,
  and a one-line description containing the short commit link, PR link, and links to
  the actual passing CI checks.
- On a failed attempt or genuine blocker, use `status: "failure"` with the actual PR,
  commit, and failed-check or conflict links when available, and explicitly say that
  the requested PR workflow did not complete. Include the conflict wording only when a
  conflict remains unresolved.

Do not report a prepared commit, a pushed branch, a created-but-unverified PR, or
pending checks as a successful landing. If the status tool is unavailable, give the
same concise result directly in the conversation.
