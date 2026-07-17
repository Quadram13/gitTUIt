# gitTUIt

gitTUIt is a terminal UI for Git/Github workflows. I am currently using this for personal projects, as well as work tasks under specific conditions. The TUI works in it's current state for my purposes, but is still under development and may experience large/breaking changes, so use this at your own risk.

At this stage, I have started adding release/version-control workflows to this repo. The end goal is to fold these scripts/cargo-make tasks into future plugins.

## Current Features

- explicit repo tracking(requires local storage)
- stage/unstage/commit changes (subject + optional multiline body)
- diff and history views for files
    - uses simple file tree implementation(influenced by gitui's filetreelist crate)
- incoming/outgoing commit comparison against upstream tracking branch
- stash manager (stash/apply/pop/drop with preview)
- GitHub pull request view (list/filter/open/checkout via Github CLI)
- async architecture
    - async git/gh execution for status, history, stash, branch, tracking, and PR workflows
    - bounded scheduler (fixed worker pool + bounded queue), request-id stale-result protection, and cancellable queued refreshes
    - heavily influenced by `gitui`/`asyncgit`, but implemented differently:
        - this TUI uses an app-level event model + custom scheduler tuned for git + GitHub CLI flows
        - `gitui` primarily uses `asyncgit` primitives and a different internal async/runtime integration

## Motivation/Inspiration

I use Arch Linux on my daily driver(insert tired joke here), and much of my desktop ecosystem consists of TUIs(cmus, yazi, etc). TUIs also harken back to the early years of computing, which is something I have become interested in recently. 

My current job uses the [conventional commit specification](https://www.conventionalcommits.org/en/v1.0.0/#specification), as well as [release-please](https://github.com/googleapis/release-please) for releases, changelogs, and other version control automation. Repo-specific scripts used alongside the [cargo-make crate](https://crates.io/crates/cargo-make/0.3.54) handle these workflows locally, but I felt that I could make this sort of automation (relatively) repo-agnostic. 

Existing TUIs like [gitUI](https://github.com/gitui-org/gitui) contain most, if not all of the features in this project(and probably implement them way better). One might also say that individual git commands in a terminal "gives" retro computing, but everyone draws the line between "vibes" and convenience somewhere. Ultimately, I decided to build this TUI from the ground up, not just as a tool for my job, but also as a personal project and something that others can potentially use in the future.

## Roadmap

Currently, this TUI can replace(at least for me) the functionality of Github Desktop.

A concrete list of upcoming changes(in order):

* refactor TUI for Lua plugins/customization
    - potential plugins for my current use include commit message building with conventional commits and merging release followup PRs from github actions

    - themes/colors/keybinds/others may be a mix of config files and plugins

    - other ideas for plugins/customization will be added here as they come up

* changelog/packaging/release setup(in progress)
    - basic github actions workflow structure is in place
    - remaining work:
        * implement local build tasks
        * enable/test actions after completing plugin refactor and local build

* async architecture(remaining):
    - current async implementation uses github as the source for the asyncgit crate, because using the crate from crates.io leads to a git2 version mismatch
        * should probably fix this in the future 
    - surface async lifecycle telemetry in UI/log output (queue depth, running jobs, cancellation/failure counters)
    - make repository browser/directory scanning and heavier file-system work fully non-blocking
    - add broader tests for async race handling/cancellation semantics under rapid selection/view changes
    - evolve lifecycle model from queued/running/idle to include explicit success/error/cancelled states in one unified job registry

These are changes/things I note that may not slot cleanly into the list:

- not sure if this app/repo contains/interacts with any sensitive info
    * should probably do a check for any security concerns

- repo setup for contribution
    * proper dependency tracking

- integration of other version control/developer platforms (e.g. GitLab)?

## Installation/Usage

To run the TUI, you will need [rust](https://rust-lang.org/), [git](https://git-scm.com/), and the [Github CLI](https://cli.github.com/) installed. Most features will still work if the Github CLI is not set up, but Github-specific actions(like PR merges) will not. It is recommended to set up your credentials for both Git and the Github CLI to avoid this.

As mentioned in the roadmap, I have not set up a way to install a binary(either thru source code or standalone installer). For now, the only option to run the TUI is to clone this repo locally, navigate to the repo root, and run:


```zsh
cargo run
```

Run with diagnostics logging:

```zsh
cargo run -- --log
```

Optional logging flags:

- `--log-file <path>` to write logs to a custom file.
- `--log-level <error|warn|info|debug|trace>` to control verbosity.
- `-l` to print diagnostics (paths and tool versions) and exit.

You can run from anywhere; repositories are explicitly added in-app.

## Release Workflow

This repo currently uses a split CI/CD + Release Please workflow:

- `.github/workflows/ci.yml`
- `.github/workflows/cd.yml`
- `.github/workflows/release-please.yml`
- `release-please-config.json`
- `.release-please-manifest.json`
- `CHANGELOG.md`

> Workflow status: automated GitHub triggers are temporarily disabled while install/plugin refactor work is prioritized. Workflows can still be run manually via `workflow_dispatch`.

Current release component naming:

- `gitTUIt` -> `gitTUIt-vX.Y.Z`

### Commit Policy

Commit header format:

`<type>!: <description>` (or `<type>: <description>`)

- Releasable types are `feat` and `fix`.
- `!` (breaking marker) is allowed only on releasable types.
- Non-releasable types are still valid and are grouped in changelog sections when a release is cut.
- `feat`/`fix` entries are reserved for commits that include staged `src/` changes.
- Commits without `src/` changes should use non-releasable types.

Multi-entry commit format:

- First entry is the commit subject.
- Optional freeform paragraphs go in the body.
- Additional change entries go at the end of the commit body as footer-style Conventional Commit lines.

### cargo-make Tasks

Install cargo-make if needed:

```bash
cargo install cargo-make
```

Release/task commands:

```bash
cargo make release-status
cargo make commit
cargo make push
cargo make pr
cargo make merge-pr
cargo make merge-release-pr
```

Notes:

- Hook installation requires manual setup (`git config core.hooksPath .githooks`).
- Task scripts are available for both Windows PowerShell and Bash (macOS/Linux).
- Chained tasks now prompt after each step finishes before moving to the next step (push -> PR -> merge).
- `merge-pr` waits/retries on required checks only; optional checks no longer cause one-shot merge gating failures.

### Security Audit

- CI runs `cargo audit` via `.github/workflows/ci.yml` using pinned `cargo-audit` with cache-backed install.
- `.cargo/audit.toml` currently ignores `RUSTSEC-2023-0071` (transitive via `asyncgit -> ssh-key -> rsa`) because no fixed upstream upgrade is available yet.
- This ignore should be removed once an upstream dependency fix is available.

### Branch Ruleset Checks (When CI Is Re-enabled)

When re-enabling required status checks on `main`, use the CI jobs below:

- `verify-lockfile` (required)
- `cargo-audit` (required)
- `buildability` matrix checks (recommended required for all configured OS targets)

`sync-lockfile` is release-automation support and is left non-required.

## Repository Tracking

- Repositories are tracked only when explicitly introduced by the user.
- Added path must be a git root (must contain `.git` directly at that folder root).
- Tracked repositories are stored in a per-user JSON file in the OS config directory.

Runtime directory contract:

- **Config**: if `GITTUIT_CONFIG_DIR` is set, that directory is used. Otherwise:
  - debug builds (for example `cargo run`) use `gitTUIt-dev` under OS config dir
  - release builds use `gitTUIt` under OS config dir
- **Data**: OS data dir + app folder (`gitTUIt` / `gitTUIt-dev`)
- **Cache**: OS cache dir + app folder (`gitTUIt` / `gitTUIt-dev`)
- **Logs**: `<config-dir>/logs/gitTUIt.log` by default
- Plugin-specific directory conventions are deferred to the plugin phase.

## License

This project is licensed under the MIT License. See `LICENSE`.
