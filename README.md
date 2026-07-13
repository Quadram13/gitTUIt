# gitTUIt

gitTUIt is a terminal UI for Git/Github workflows.

## Current Features

- explicit repo tracking(requires local storage)
- stage/unstage/commit changes 
- diff preview for selected file
- history view with commit details, checkout (detached), and cherry-pick
- incoming/outgoing commit comparison against upstream tracking branch
- stash manager (stash/apply/pop/drop with preview)
- GitHub pull request view (list/filter/open/checkout via Github CLI)

## Motivation/Inspiration

I use Arch Linux on my daily driver(insert tired joke here), and much of my desktop ecosystem consists of TUIs(cmus, yazi, etc). TUIs also harken back to the early years of computing, which is something I have become interested in recently. 

My current job uses the [conventional commit specification](https://www.conventionalcommits.org/en/v1.0.0/#specification), as well as [release-please](https://github.com/googleapis/release-please) for releases, changelogs, and other version control automation. Repo-specific scripts used alongside the [cargo-make crate](https://crates.io/crates/cargo-make/0.3.54) handle these workflows locally, but I felt that I could make this sort of automation (relatively) repo-agnostic. 

Existing TUIs like [gitui](https://github.com/gitui-org/gitui) contain most, if not all of the features in this project(and probably implement them way better). One might also say that individual git commands in a terminal "gives" retro computing, but everyone draws the line between "vibes" and convenience somewhere. 

Ultimately, I decided to build this TUI from the ground up, not just as a tool for my job, but also as a personal project and something that others can potentially use in the future.

## Roadmap

Currently, this TUI can replace(at least for me) the functionality of Github Desktop.

A concrete list of upcoming changes(in order):

* changelog/packaging/release setup

* plugin implementation
    - potential plugins for my current use include commit message building with conventional commits and merging release followup PRs from github actions

    - other ideas for plugins will be added here as they come up

* customization(themes, colors, syntax highlighting, keybinds)

These are changes/things I note that may not slot cleanly into the list:

- not sure if this app/repo contains/interacts with any sensitive info
    * should probably do a check for any security concerns

- most established TUIs use async features/IO
    * this TUI currently does not have any async stuff implemented, but works fine for now

    * should probably add this at some point

- repo setup for contribution
    * proper license(probably MIT?)

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

## Repository Tracking

- Repositories are tracked only when explicitly introduced by the user.
- Added path must be a git root (must contain `.git` directly at that folder root).
- Tracked repositories are stored in a per-user JSON file in the OS config directory.

Config directory resolution:

- If `GITTUIT_CONFIG_DIR` is set, that directory is used for `repos.json`.
- Otherwise, debug builds (for example `cargo run`) use `gitTUIt-dev` under the OS config directory.
- Release builds use `gitTUIt` under the OS config directory.
