# BKSD - Backup Sentinel Daemon

A Rust daemon that automatically detects removable storage devices (SD cards, USB drives) and backs up their contents to a configured destination.

## Features

- Automatic device detection via udev
- Supports ext4, exfat, vfat (FAT32/FAT16), ntfs, and btrfs filesystems
- Uses rsync for efficient incremental backups
- SQLite database for job tracking and history
- JSON-RPC 2.0 API for querying status and progress
- Safe device cleanup with filesystem sync before unmount

## Requirements

- Linux (uses udev for device monitoring)
- Root privileges (for mounting devices)
- rsync (for file transfers)

## Installation

```bash
cargo build --release
sudo cp target/release/bksd /usr/local/bin/
```

## Usage

### Running the Daemon

The daemon requires root privileges to mount devices and a backup directory:

```bash
sudo bksd daemon -d /mnt/backups
```

With a custom mount location:

```bash
sudo bksd daemon -d /mnt/backups -m /mnt/bksd
```

### Configuration Options

| Short | Long | Environment Variable | Default | Description |
|-------|------|---------------------|---------|-------------|
| `-d` | `--backup-directory` | `BKSD_BACKUP_DIRECTORY` | **required** | Where backups are stored |
| `-m` | `--mount-base` | `BKSD_MOUNT_BASE` | `/run/bksd` | Where devices are mounted |
| `-e` | `--transfer-engine` | `BKSD_TRANSFER_ENGINE` | `rsync` | Transfer engine (`rsync` or `simulated`) |
| `-r` | `--retry-attempts` | `BKSD_RETRY_ATTEMPTS` | `3` | Number of retry attempts on failure |
| `-s` | `--simulation` | `BKSD_SIMULATION` | `false` | Use simulated hardware adapter |
| `-v` | `--verbose` | `BKSD_VERBOSE` | `false` | Enable verbose output |
| | | `BKSD_RPC_ENABLED` | `true` | Enable the RPC server |
| | | `BKSD_RPC_BIND` | `127.0.0.1:9847` | RPC server bind address |

### Simulation Mode

For testing without real devices, use simulation mode:

```bash
bksd daemon -d /tmp/test-backups -s true
```

Then type commands to simulate device events:

```
add my-sd-card     # Simulate inserting a device with UUID "my-sd-card"
rm my-sd-card      # Simulate removing the device
add                # Uses default UUID "123"
rm                 # Removes default UUID "123"
```

### Checking Status

Query the running daemon for status and active jobs:

```bash
bksd status
```

Example output:

```
Daemon Status
  Version:     0.1.0
  Uptime:      42s
  Mode:        simulation
  Active Jobs: 1

Active Transfers:
  019482ab - 67% - DCIM/IMG_0001.CR3
```

Connect to a daemon on a different address:

```bash
bksd status --addr 192.168.1.100:9847
```

## RPC API

The daemon exposes a JSON-RPC 2.0 API over TCP for querying job status and progress. By default, it listens on `127.0.0.1:9847`.

### Protocol

- **Transport**: TCP with newline-delimited JSON
- **Format**: JSON-RPC 2.0

Each request/response is a single line of JSON terminated by `\n`.

### Example Session

```bash
# Connect to the daemon
nc localhost 9847

# Send a request (single line)
{"jsonrpc":"2.0","method":"daemon.status","id":1}

# Response
{"jsonrpc":"2.0","result":{"version":"0.1.0","uptime_secs":120,"active_jobs":1,"rpc_bind":"127.0.0.1:9847","simulation":false},"id":1}
```

### Available Methods

#### `daemon.status`

Get daemon health and status information.

**Parameters**: None

**Response**:
```json
{
  "version": "0.1.0",
  "uptime_secs": 120,
  "active_jobs": 1,
  "rpc_bind": "127.0.0.1:9847",
  "simulation": false
}
```

#### `jobs.list`

List backup jobs with optional filtering and pagination.

**Parameters**:
| Name | Type | Required | Description |
|------|------|----------|-------------|
| `limit` | integer | No | Max jobs to return (default: 50) |
| `offset` | integer | No | Number of jobs to skip (default: 0) |
| `status` | string | No | Filter by status (e.g., "Complete", "Failed") |

**Example Request**:
```json
{"jsonrpc":"2.0","method":"jobs.list","params":{"limit":10,"status":"Complete"},"id":1}
```

**Response**:
```json
[
  {
    "id": "019482ab-...",
    "target_id": "device-uuid",
    "destination_path": "/mnt/backups/CANON_SD/2024-01-10_T1530_00",
    "created_at": "2024-01-10T15:30:00",
    "status": "Complete"
  }
]
```

#### `jobs.get`

Get a single job with its full status history.

**Parameters**:
| Name | Type | Required | Description |
|------|------|----------|-------------|
| `id` | string | Yes | Job ID |

**Example Request**:
```json
{"jsonrpc":"2.0","method":"jobs.get","params":{"id":"019482ab-..."},"id":1}
```

**Response**:
```json
{
  "id": "019482ab-...",
  "target_id": "device-uuid",
  "destination_path": "/mnt/backups/CANON_SD/2024-01-10_T1530_00",
  "created_at": "2024-01-10T15:30:00",
  "status": "Complete",
  "history": [
    {"id": "...", "status": "Ready", "description": "Job created", "created_at": "..."},
    {"id": "...", "status": "InProgress", "description": "Transfer started", "created_at": "..."},
    {"id": "...", "status": "Complete", "total_bytes": 1073741824, "duration_secs": 120, "created_at": "..."}
  ]
}
```

#### `progress.active`

Get all currently active jobs with their live transfer progress.

**Parameters**: None

**Response**:
```json
{
  "jobs": {
    "019482ab-...": {
      "state": "in_progress",
      "total_bytes": 1073741824,
      "bytes_copied": 536870912,
      "current_file": "DCIM/IMG_0042.CR3",
      "percentage": 50
    }
  },
  "count": 1
}
```

#### `progress.get`

Get live progress for a specific active job.

**Parameters**:
| Name | Type | Required | Description |
|------|------|----------|-------------|
| `id` | string | Yes | Job ID |

**Example Request**:
```json
{"jsonrpc":"2.0","method":"progress.get","params":{"id":"019482ab-..."},"id":1}
```

**Response** (varies by state):
```json
{
  "state": "in_progress",
  "total_bytes": 1073741824,
  "bytes_copied": 536870912,
  "current_file": "DCIM/IMG_0042.CR3",
  "percentage": 50
}
```

### Transfer Status States

The `progress.get` and `progress.active` methods return status objects with a `state` field:

| State | Fields | Description |
|-------|--------|-------------|
| `ready` | - | Job created, waiting to start |
| `in_progress` | `total_bytes`, `bytes_copied`, `current_file`, `percentage` | Transfer in progress |
| `copy_complete` | - | Files copied, preparing for verification |
| `verifying` | `current`, `total` | Verifying transferred files |
| `complete` | `total_bytes`, `duration_secs` | Transfer completed successfully |
| `failed` | (error message as string) | Transfer failed |

### Error Codes

Standard JSON-RPC 2.0 error codes:

| Code | Meaning |
|------|---------|
| -32700 | Parse error - invalid JSON |
| -32600 | Invalid request - missing required fields |
| -32601 | Method not found |
| -32602 | Invalid params |
| -32603 | Internal error |
| -32000 | Application error (e.g., job not found) |

## Architecture

```
                    ┌─────────────────────┐
                    │   bksd status CLI   │
                    └──────────┬──────────┘
                               │ TCP (JSON-RPC 2.0)
                               ▼
┌──────────────────────────────────────────────────────┐
│                    bksd daemon                        │
│  ┌─────────────┐     ┌─────────────────────────────┐ │
│  │ Orchestrator│     │        RpcServer            │ │
│  │             │◄────┤  - Transport (TCP)          │ │
│  │  AppContext │     │  - MethodHandler            │ │
│  │   - DB      │     │    - daemon.status          │ │
│  │   - Progress│     │    - jobs.list/get          │ │
│  └─────────────┘     │    - progress.active/get    │ │
│                      └─────────────────────────────┘ │
└──────────────────────────────────────────────────────┘
```

### Components

- **Orchestrator**: Central coordinator that listens for hardware events and manages backup jobs
- **RpcServer**: JSON-RPC 2.0 server for client communication
- **ProgressTracker**: In-memory store for live transfer progress (updated every tick, not persisted)
- **Database**: SQLite for job history (only state transitions are persisted)

### Progress Tracking

To avoid database bloat, progress updates are handled differently:

- **In-memory**: Every progress tick updates the `ProgressTracker` (for real-time queries)
- **Database**: Only state transitions are persisted (Ready → InProgress → Complete)

A typical completed job has ~5 database rows instead of thousands.

## How It Works

1. **Device Detection**: The daemon monitors udev for block device events
2. **Mounting**: When a supported device is inserted, it's mounted to `/run/bksd/<uuid>`
3. **Backup**: Contents are copied to `<backup-directory>/<label>/<timestamp>/`
4. **Cleanup**: After backup, the filesystem is synced and unmounted

### Backup Directory Structure

```
/mnt/backups/
  CANON_SD/
    2024-01-10_T1530_00/
      DCIM/
      ...
    2024-01-11_T0900_00/
      DCIM/
      ...
  GOPRO/
    2024-01-10_T1600_00/
      ...
```

### File Ownership

Since the daemon runs as root, backed up files could end up owned by root and inaccessible to normal users. To prevent this, bksd automatically detects the appropriate owner for backup files:

1. **`SUDO_USER`** (preferred): When you run `sudo bksd`, the daemon detects the original user from the `SUDO_USER` environment variable and sets file ownership accordingly.

2. **Backup directory owner** (fallback): If `SUDO_USER` is not set (e.g., when running as a systemd service), the daemon uses the owner of the backup directory.

**Example**: If user `joshua` runs `sudo bksd daemon -d /mnt/backups`, all backed up files will be owned by `joshua:joshua`.

**For systemd services**: Ensure your backup directory is owned by the desired user:

```bash
sudo mkdir -p /mnt/backups
sudo chown joshua:joshua /mnt/backups
```

## Running as a systemd Service

Create `/etc/systemd/system/bksd.service`:

```ini
[Unit]
Description=Backup Sentinel Daemon
After=network.target

[Service]
Type=simple
ExecStart=/usr/local/bin/bksd daemon -d /mnt/backups
Restart=on-failure
RuntimeDirectory=bksd

[Install]
WantedBy=multi-user.target
```

Enable and start:

```bash
sudo systemctl enable bksd
sudo systemctl start bksd
```

## Development

### Running Tests

```bash
# Run all tests (no root required)
cargo test

# Run Linux adapter tests requiring root
sudo cargo test --test linux_adapter -- --ignored
```

### Project Structure

```
src/
  adapters/       # Hardware detection (LinuxAdapter, SimulatedAdapter)
  core/
    orchestrator.rs    # Main coordinator
    hardware.rs        # Device types and traits
    progress.rs        # In-memory progress tracking
    transfer_engine/   # Backup engines (rsync, simulated)
  config.rs       # Configuration handling
  db/             # SQLite job persistence
  rpc/
    mod.rs             # RpcServer
    protocol.rs        # JSON-RPC 2.0 types
    transport.rs       # TCP listener and framing
    methods.rs         # Method handlers
    client.rs          # RpcClient for CLI
tests/
  linux_adapter.rs      # Linux adapter integration tests
  simulated_adapter.rs  # Simulated adapter tests
```

### Extending the RPC API

To add a new method:

1. Add a handler method in `src/rpc/methods.rs`
2. Add a match arm in `MethodHandler::handle()`

Example:

```rust
// In methods.rs
async fn my_new_method(&self, id: Value, params: Value) -> Response {
    // Implementation
    Response::success(id, result)
}

// In handle()
match request.method.as_str() {
    // ...existing methods...
    "my.newMethod" => self.my_new_method(id, params).await,
    _ => Response::method_not_found(id, &request.method),
}
```

### Future Extensibility

The RPC architecture supports:

- **Push notifications**: Server can send JSON-RPC notifications to connected clients
- **Subscriptions**: Add `subscribe.*` methods for real-time event streaming
- **Authentication**: Add token validation in transport layer for remote access

## License

MIT
