# slurm-monitor

A small always-on-top overlay that watches your SLURM jobs on a remote
cluster over SSH and notifies you when their state changes.

## Setup

1. On first run, use the in-app **Settings** panel (see below) to add
   one or more clusters — no manual file editing required. If you
   prefer to hand-edit a config file, see "Configuration" below for its
   location and shape.
2. **If your private key is Ed25519** (`id_ed25519`), you need OpenSSL
   available at build time — see "Ed25519 keys and OpenSSL" below. RSA
   and ECDSA keys work with no extra setup.
3. `cd src-tauri && cargo build`
4. `cd src-tauri && cargo run`

## Ed25519 keys and OpenSSL

By default this app's SSH library (`ssh2`/`libssh2`) builds against
Windows' native CNG crypto backend, which has **no Ed25519 support** —
an Ed25519 key will fail auth with a generic
`[Session(-1)] unknown error` in the log. RSA/ECDSA keys work fine
without any of this.

To use an Ed25519 key, this project is configured (`Cargo.toml`'s
`ssh2 = { features = ["openssl-on-win32"] }`) to link against a real
OpenSSL instead, located via `src-tauri/.cargo/config.toml`. That
requires OpenSSL installed once via vcpkg:

```
git clone https://github.com/microsoft/vcpkg C:/vcpkg
C:/vcpkg/bootstrap-vcpkg.bat
C:/vcpkg/vcpkg.exe install openssl:x64-windows-static-md
```

If you install vcpkg somewhere other than `C:/vcpkg`, update the path
in `src-tauri/.cargo/config.toml` to match. After that, `cargo build`
picks up OpenSSL automatically — no manual env vars needed.

## Configuration (`config.toml`)

Config no longer lives next to the executable. It's read from and
written to the OS-specific **app config directory** (Tauri's
`app_config_dir()`, derived from the app identifier
`sk.kinit.perun.slurm-monitor`):

| OS      | Path                                                                 |
|---------|-----------------------------------------------------------------------|
| Windows | `%APPDATA%\sk.kinit.perun.slurm-monitor\config.toml`                  |
| macOS   | `~/Library/Application Support/sk.kinit.perun.slurm-monitor/config.toml` |
| Linux   | `~/.config/sk.kinit.perun.slurm-monitor/config.toml`                  |

In each case the file loaded/saved is `config.toml` inside that
directory.

### Legacy config auto-migration

If no config exists yet at the app-config path but a `config.toml` is
found in the current working directory (the old single-cluster
location, e.g. `src-tauri/config.toml` under `cargo run`), it is
automatically read, converted into the new multi-cluster shape as a
single cluster named `"default"`, and saved to the new app-config
location. No manual conversion is needed — this happens transparently
on first launch after upgrading.

### `[[clusters]]` array

The config is now a TOML array of tables, one per cluster:

| Field                | Required | Default            | Description                                   |
|----------------------|----------|---------------------|------------------------------------------------|
| `name`               | yes      | —                   | Unique display name for the cluster            |
| `host`               | yes      | —                   | SSH host of the cluster login node             |
| `port`               | no       | `22`                | SSH port                                       |
| `username`           | yes      | —                   | SSH username                                   |
| `key_path`           | yes      | —                   | Path to the SSH private key                    |
| `key_passphrase`     | no       | none                | Passphrase for the private key, if any         |
| `poll_interval_secs` | no       | `60`                | Seconds between `squeue` polls for this cluster |
| `squeue_user`        | no       | same as `username`  | SLURM user to track jobs for                   |

Example:

```toml
[[clusters]]
name = "devana"
host = "login.devana.example"
username = "jdoe"
key_path = "/home/jdoe/.ssh/id_ed25519"

[[clusters]]
name = "lumi"
host = "lumi.example"
port = 2222
username = "ivan"
key_path = "/home/jdoe/.ssh/id_rsa"
poll_interval_secs = 30
```

Each cluster gets its own supervised poller and connection-status
badge in the overlay; a failure on one cluster does not affect others.

### `[notifications]` block

An optional top-level table controlling notification behavior across
all clusters:

| Field           | Required | Default        | Description                                                   |
|-----------------|----------|-----------------|-----------------------------------------------------------------|
| `notify_states` | no       | `[]` (all)      | List of job states to notify on (e.g. `["COMPLETED", "FAILED"]`); empty means notify on every transition |
| `quiet_start`   | no       | none            | Start of a daily quiet period, `"HH:MM"`, during which notifications are suppressed |
| `quiet_end`     | no       | none            | End of the daily quiet period, `"HH:MM"`                        |

Example:

```toml
[notifications]
notify_states = ["COMPLETED", "FAILED", "CANCELLED"]
quiet_start = "22:00"
quiet_end = "07:00"
```

### In-app Settings UI

Clusters and notification preferences no longer require hand-editing
TOML at all: the overlay has a **Settings** panel (accessible from the
UI) for adding, editing, and removing clusters and for changing
notification preferences. Saving in Settings writes the config file
via the `save_config` command and live-restarts the affected
per-cluster pollers — no app restart needed. Editing the config file
externally is also picked up automatically: the app watches the
config file and hot-reloads on external changes.

## Actions

Beyond passive monitoring, the overlay supports:

- **Cancel a job** — issues `scancel <job_id>` on the job's cluster
  directly from the job list.
- **Peek log output** — resolves the job's `StdOut` path via
  `scontrol show job <job_id>` and tails the last lines of the log
  file for a quick look without opening a separate SSH session.
- **Cluster health** — shows per-cluster partition availability (via
  `sinfo`) so you can see node/partition status alongside job state.

## Notes

- Jobs are tracked via `squeue -u <user>`, once per configured
  cluster; once a job leaves the queue, its final state
  (COMPLETED/FAILED/CANCELLED/...) is resolved with a `sacct` lookup.
- Logs are written daily to the app's log directory
  (`tracing_appender::rolling::daily`).
