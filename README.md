# git-trano

Git plugin written in Rust for **Capistrano-style** deployments from the current repository.

It allows you to:

- Sync remote references (`git fetch`)
- Deploy a **branch** or a **tag**
- Create releases under `releases/<timestamp>`
- Update the `current` symlink
- Keep only the latest _N_ releases
- Revert `current` to the previous release
- Manage shared path symlinks (`--shared`)

---

## Features

- Plugin-style command: `git trano ...`
- Deployment layout:
  - `<path>/releases`
  - `<path>/current` (symlink)
  - `<path>/shared`
- Atomic deployment by switching symlink
- Automatic cleanup of old releases (`--keep`, default `3`)
- Multiple `--shared` entries supported
- Makefile with build, static build, and install tasks
- GitHub Actions pipeline to build static binaries (musl) and publish them to releases
- Linux/macOS compatible (POSIX symlinks)

---

## Generated destination structure

Given `--path /www/folder`, this is created:

```/dev/null/tree.txt#L1-8
/www/folder/
├── current -> /www/folder/releases/2026-01-01T12-00-00Z
├── releases/
│   ├── 2026-01-01T11-00-00Z/
│   └── 2026-01-01T12-00-00Z/
└── shared/
    ├── node_modules/
    └── vendor/subfolder/
```

---

## Requirements

- Git installed and available in `PATH`
- Stable Rust toolchain (if building from source)
- Must be run inside a valid Git repository

---

## Installation

### Option 1: Build from source (cargo)

```/dev/null/install-build.sh#L1-4
cargo build --release
install -m 0755 target/release/git-trano /usr/local/bin/git-trano
# optional plugin alias:
ln -sf /usr/local/bin/git-trano /usr/local/bin/git-trano
```

Then you can run:

```/dev/null/usage.txt#L1-1
git trano ...
```

> Git invokes subcommands through executables named `git-<subcommand>`.  
> For `git trano`, the executable must be named `git-trano`.

### Option 2: Local `cargo install`

```/dev/null/cargo-install.sh#L1-1
cargo install --path .
```

Make sure `~/.cargo/bin` is in your `PATH`.

### Option 3: Use Makefile

```/dev/null/make-quickstart.sh#L1-6
make help
make build
make release
make static
make static-all
make install
```

---

## Usage

```/dev/null/help.txt#L1-14
git trano --branch <branch_name> --path <destination_path> [--keep <n>] [--shared <path>]...
git trano --tag <tag> --path <destination_path> [--keep <n>] [--shared <path>]...
git trano --revert --path <destination_path>

Options:
  -b, --branch <branch>   Deploy the specified branch
  -t, --tag <tag>         Deploy the specified tag
  -p, --path <path>       Deployment base path
  -k, --keep <n>          Releases to keep (default: 3)
      --shared <path>     Shared path (repeatable)
  -r, --revert            Point current to the previous release
  -h, --help              Help
  -V, --version           Version
```

---

## Examples

### Deploy a branch

```/dev/null/examples.txt#L1-2
git trano --branch main --path /www/folder --keep 5
git trano -b main -p /www/folder
```

What it does:

1. `git fetch --all --prune`
2. Updates local checkout to the requested branch
3. Copies current working directory to:
   - `/www/folder/releases/<timestamp>`
4. Replaces symlink:
   - `/www/folder/current -> /www/folder/releases/<timestamp>`
5. Removes old releases and keeps the latest `N`

### Deploy a tag

```/dev/null/examples.txt#L3-4
git trano --tag v1.2.3 --path /www/folder
git trano -t v1.2.3 -p /www/folder
```

Same flow as branch deploy, but with tag checkout.

### Revert to previous release

```/dev/null/examples.txt#L6-6
git trano --revert --path /www/folder
```

- Reads releases sorted by timestamp
- Points `current` to the previous available release

### Shared links

```/dev/null/examples.txt#L8-8
git trano --branch main --path /www/folder --shared node_modules --shared vendor/subfolder
```

After updating `current`:

- ensures `/www/folder/shared/node_modules`
- ensures `/www/folder/shared/vendor/subfolder`
- removes existing paths in `current` if present
- creates symlinks:
  - `/www/folder/current/node_modules -> /www/folder/shared/node_modules`
  - `/www/folder/current/vendor/subfolder -> /www/folder/shared/vendor/subfolder`

---

## Internal flow (summary)

1. Argument validation (branch/tag/revert are mutually exclusive)
2. Base layout preparation (`releases`, `shared`)
3. Deploy mode:
   - fetch remotes
   - checkout branch/tag
   - create timestamped release
   - copy repository files to release (excluding `.git`)
   - update `current` symlink
   - apply `shared` links
   - cleanup old releases
4. Revert mode:
   - list releases
   - move `current` to previous release

---

## Static build

> Note: on Linux glibc, binaries are usually dynamically linked by default.  
> For a true static binary, use **musl**.

### Linux x86_64 static (musl)

```/dev/null/static-build.sh#L1-3
rustup target add x86_64-unknown-linux-musl
cargo build --release --target x86_64-unknown-linux-musl
strip target/x86_64-unknown-linux-musl/release/git-trano
```

Resulting binary:

```/dev/null/static-build.sh#L5-5
target/x86_64-unknown-linux-musl/release/git-trano
```

Verify it is static:

```/dev/null/static-check.sh#L1-1
ldd target/x86_64-unknown-linux-musl/release/git-trano
```

It should report that it is not a dynamic executable (or equivalent).

### Cross-compilation (optional with `cross`)

```/dev/null/cross.sh#L1-2
cargo install cross
cross build --release --target x86_64-unknown-linux-musl
```

## Included Makefile

The project includes a `Makefile` with the main targets:

```/dev/null/make-targets.txt#L1-13
make help
make check
make fmt
make clippy
make test
make build
make release
make static
make static-x86_64
make static-aarch64
make static-all
make install
make uninstall
```

Notes:
- `make static` builds `x86_64-unknown-linux-musl`.
- `make static-all` builds both `x86_64` and `aarch64` musl binaries.
- `make install` installs by default to `/usr/local/bin/git-trano`.
- You can customize install paths with `PREFIX`, `BINDIR`, and `DESTDIR`.

Example:

```/dev/null/make-install-example.sh#L1-1
make install PREFIX=/usr DESTDIR=/tmp/pkgroot
```

## GitHub Actions pipeline for static binaries

Included workflow:

- `.github/workflows/release-static.yml`

What it does:
1. Runs on tags `v*` (and manually via `workflow_dispatch`).
2. Builds static `musl` binaries for:
   - `x86_64-unknown-linux-musl`
   - `aarch64-unknown-linux-musl`
3. Packages each binary in `.tar.gz`.
4. Generates `.sha256` checksum per artifact.
5. Uploads workflow artifacts.
6. If triggered by a tag, uploads assets to the GitHub Release.
7. Also generates a combined `SHA256SUMS` file and attaches it to the release.

### How to publish a static release

```/dev/null/release-flow.sh#L1-7
git tag v0.1.0
git push origin v0.1.0
# GitHub Actions will build and attach:
# - git-trano-v0.1.0-linux-amd64-musl.tar.gz
# - git-trano-v0.1.0-linux-amd64-musl.tar.gz.sha256
# - git-trano-v0.1.0-linux-arm64-musl.tar.gz
# - git-trano-v0.1.0-linux-arm64-musl.tar.gz.sha256
```

---

## Best practices

- Run `git trano` from the correct repository (clean working tree recommended)
- For production, use a restricted user with limited permissions over `--path`
- Verify disk space if you increase `--keep`
- Use `--shared` for persistent data across releases (`uploads`, `storage`, `node_modules`, etc.)
- If deploying tags, prefer immutable signed tags

---

## Troubleshooting

### `git trano` command not found

Make sure the binary is installed as `git-trano` and is in your `PATH`:

```/dev/null/troubleshoot-notfound.sh#L1-2
which git-trano
git trano --help
```

### `Not inside a git working tree`

Run the command from inside a valid Git repository:

```/dev/null/troubleshoot-gitrepo.sh#L1-1
git rev-parse --is-inside-work-tree
```

### `Failed to create directory ... File name too long (os error 36)`

This typically indicates recursive copying into nested release paths.

Use a deploy path outside the repository contents (recommended), for example:

```/dev/null/troubleshoot-path.sh#L1-1
git trano -b main -p /www/gitrano_releases --keep 4
```

Also ensure the tool excludes the deployment base directory from the source copy set.

### Revert does not work

`--revert` requires at least two releases in `<path>/releases`.

Check:

```/dev/null/troubleshoot-revert.sh#L1-1
ls -1 /www/folder/releases
```

---

## License

See `LICENSE`.