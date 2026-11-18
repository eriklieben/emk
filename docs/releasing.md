# Releasing

Releases are made by [semantic-release](https://github.com/semantic-release/semantic-release) from the commit
messages on `main`: nobody picks a version, edits the changelog or pushes a tag by hand. How to write the commits:
[development.md, "Commit messages"](development.md#commit-messages).

| File | What |
| --- | --- |
| [`release.config.mjs`](../release.config.mjs) | Branches, tag format `v<version>`, plugins, release notes, release assets. |
| [`eng/release-commit.mjs`](../eng/release-commit.mjs) | The release commit through the GitHub API (signed, *Verified*). Tests: `node --test eng/`. |
| [`.github/workflows/release.yml`](../.github/workflows/release.yml) | On a push to `main`: CI, the release builds, then semantic-release. |
| [`commitlint.config.mjs`](../commitlint.config.mjs) | The commit message rules (local `commit-msg` hook). |
| [`CHANGELOG.md`](../CHANGELOG.md) | Written by the release, never by hand. |

## Versioning

EMK follows [Semantic Versioning](https://semver.org) with semantic-release's default rules:

| Commit | Release |
| --- | --- |
| `fix: ...`, `perf: ...`, `revert: ...` | patch: 1.4.2 → 1.4.3 |
| `feat: ...` | minor: 1.4.2 → 1.5.0 |
| `feat!: ...` / `BREAKING CHANGE:` footer | major: 1.4.2 → 2.0.0 |
| `docs`, `chore`, `refactor`, `test`, `ci`, `build`, `style` | no release |

A breaking change is one that makes users redo something: a keymap or settings format that no longer loads, a host
protocol change that needs a new kbnotify or app, a flash layout that loses the bonds. Say what to do in the
`BREAKING CHANGE:` footer. `CHANGELOG.md` and the GitHub release get the same notes: *⚠ BREAKING CHANGES*,
*Features*, *Bug Fixes*, *Performance*, *Reverts* and *Dependencies*.

## How a release works

1. A commit reaches `main`. The **Release** workflow (`release.yml`) runs CI (`ci.yml`, as a reusable workflow),
   then builds the release files: the Bluetooth split firmware for both halves of `erik_kbd`
   (`fw-nrf-left-ble-erik_kbd.uf2`, `fw-nrf-right-ble-erik_kbd.uf2`, unsigned: the signing key stays on the
   maintainer's machine) and kbnotify as single files for Linux and Windows. It checks that all four are there and
   writes `SHA256SUMS`.
2. semantic-release looks at the commits since the last tag `v*`. No `feat`, `fix`, `perf`, `revert` or breaking
   change: nothing happens.
3. Otherwise it writes the release notes to the top of `CHANGELOG.md` and the **emk-release** GitHub App commits it
   through the GitHub API (`createCommitOnBranch`, with the released commit as `expectedHeadOid`) as
   `chore(release): <version> [skip ci]`. GitHub signs that commit, so it shows as *Verified*, which `main`'s
   ruleset requires; `[skip ci]` keeps it from starting another run. If `main` moved on in the meantime, GitHub
   refuses the commit and nothing is tagged: the run of the newer commit releases both.
4. The commit is tagged `v<version>` (pushed with the App's token) and the GitHub release is created with the notes,
   the four files and `SHA256SUMS`. Released pull requests and issues get a comment and the label `released`.
5. Once the repository is public, the files also get a signed build provenance attestation:
   `gh attestation verify <file> --repo eriklieben/emk`. While it's private this step is skipped.

Without the App (no variable `RELEASE_APP_ID`) there is no release commit at all, never an unverified push to
`main`: the tag goes on the released commit and the notes are only on the GitHub release.

Don't edit `CHANGELOG.md` or create `v*` tags by hand.

## Setup (once)

- **GitHub App** `emk-release`: permission *Contents: read and write*, installed on this repository only. Its App ID
  is the repository variable `RELEASE_APP_ID`.
- **Environment** `release` (Settings → Environments): deployment branches limited to `main`; it holds the App's
  private key as the secret `RELEASE_APP_PRIVATE_KEY`. Only the release job uses this environment. If the variable is
  set but the key is missing, the job stops with an error instead of releasing without the commit.
- **Rulesets:** `main branch` (no deletion, no force push, signed commits required) and `release tags` (`v*`: only
  the App may create, update or delete them, as the ruleset's bypass actor; added once the App is installed). The release job's own `GITHUB_TOKEN`
  only creates the GitHub release and the comments.
- **SonarQube Cloud** (optional, CI's `sonar` job, which also runs for `main` through the Release workflow): repository
  variable `SONAR_ORGANIZATION` (and `SONAR_PROJECT_KEY` if it isn't `eriklieben_emk`) and the secret `SONAR_TOKEN`.
  See [development.md, "Code analysis"](development.md#code-analysis).

## Prereleases

`release.config.mjs` also knows the branches `next` and `beta`, which release prereleases such as `1.3.0-beta.1` (GitHub
prereleases). `release.yml` runs on `main` only and the `release` environment allows `main` only, so add the branch to
both before using one. Merge it into `main` with a merge commit (not a squash) to release `1.3.0`.

## Dry run

- **GitHub:** Actions → **Release** → *Run workflow* → branch `main`, *Dry run* ticked (the default). It runs CI and
  the builds, then prints the next version and its notes; nothing is committed, tagged or published.
- **Local** (needs push access; the `https` URL avoids a `git push --dry-run` over SSH that waits for a key):

  ```sh
  pnpm install
  GITHUB_TOKEN=$(gh auth token) pnpm exec semantic-release --dry-run --no-ci \
    --repository-url https://github.com/eriklieben/emk.git
  ```
