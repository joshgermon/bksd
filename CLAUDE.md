# BKSD - Backup Sentinel Daemon

## Overview

BKSD is a Rust backup daemon that automatically detects removable storage devices (SD cards, USB drives) and backs up their contents to a configured destination directory.

## Architecture

### Core Components

**Orchestrator** (`src/core/orchestrator.rs`)
- Central coordinator that listens for hardware events and triggers backup jobs
- Spawns transfer tasks and monitors progress
- Persists job status to SQLite database (state transitions only)
- Updates in-memory ProgressTracker for live progress

**Hardware Adapters** (`src/adapters/`)
- Trait-based system (`HardwareAdapter`) for detecting storage devices
- `LinuxAdapter`: Uses udev to monitor block device add/remove events
- `SimulatedAdapter`: For testing, accepts stdin commands (`add <uuid>`, `rm <uuid>`)

**Transfer Engines** (`src/core/transfer_engine/`)
- Trait-based system (`TransferEngine`) for copying data
- `RsyncEngine`: Uses external rsync with progress parsing
- `SimulatedEngine`: Mock implementation for testing

**Progress Tracker** (`src/core/progress.rs`)
- Thread-safe in-memory store for live transfer progress
- Updated on every progress tick from transfer engines
- Queryable via RPC for real-time status

**Verifier** (`src/core/verifier.rs`)
- Post-transfer integrity verification using BLAKE3 checksums
- Compares all files in source vs destination byte-for-byte
- Sequential file processing to avoid overwhelming slow storage devices
- Collects all mismatches before reporting failure
- Configurable via `verify_transfers` config option
- Skipped in simulation mode

**RPC Server** (`src/rpc/`)
- JSON-RPC 2.0 server over TCP (default: `127.0.0.1:9847`)
- Methods: `daemon.status`, `jobs.list`, `jobs.get`, `progress.active`, `progress.get`
- Used by `bksd status` CLI command

### Key Types

```rust
// Hardware events sent via tokio mpsc channels
enum HardwareEvent {
    DeviceAdded(BlockDevice),
    DeviceRemoved(String),  // uuid
}

// Represents a detected storage device
struct BlockDevice {
    uuid: String,
    label: String,
    path: PathBuf,        // /dev/sdb1
    mount_point: PathBuf, // /run/bksd/<uuid>
    capacity: u64,
    filesystem: String,   // ext4, vfat, exfat, ntfs, btrfs
}

// Transfer progress states
enum TransferStatus {
    Ready,
    InProgress { total_bytes, bytes_copied, current_file, percentage },
    CopyComplete,
    Verifying { current, total },
    Complete { total_bytes, duration_secs },
    Failed(String),
}
```

### RPC Module Structure

```
src/rpc/
  mod.rs         # RpcServer struct, module exports
  protocol.rs    # JSON-RPC 2.0 types (Request, Response, RpcError)
  transport.rs   # TCP listener with newline-delimited JSON framing
  methods.rs     # Method dispatcher and handlers
  client.rs      # RpcClient for CLI status command
```

**Adding a new RPC method:**
1. Add handler in `methods.rs`
2. Add match arm in `MethodHandler::handle()`

### Linux Adapter Details

The Linux adapter uses a two-thread architecture due to udev types not being Send/Sync:

1. **Blocking udev thread**: Monitors `/dev` via udev netlink socket, uses `nix::poll` with 500ms timeout for cancellation checks
2. **Async event processor**: Receives extracted event data via channel, handles mounting

**Supported filesystems**: ext4, exfat, vfat (fat32/fat16), ntfs, btrfs

**Mount behavior**:
- Checks if device is already mounted via `/proc/mounts`
- Auto-mounts to `/run/bksd/<uuid>` if not mounted
- Tracks which devices we mounted vs. system-mounted

**Cleanup**: Syncs filesystem via `syncfs()`, unmounts with `MNT_DETACH` if we mounted it

### Progress Tracking Architecture

To avoid database bloat, progress is tracked at two levels:

1. **In-memory (ProgressTracker)**: Updated every tick, for real-time queries
2. **Database (job_status_log)**: Only state transitions persisted

A completed job has ~5 database rows instead of thousands.

## Build & Run

```bash
cargo build
cargo run -- daemon -d /tmp/backups -s true  # Simulation mode
cargo run -- status                          # Query running daemon
```

## Configuration

Configuration via environment variables:
- `BKSD_BACKUP_DIRECTORY`: Destination for backups
- `BKSD_TRANSFER_ENGINE`: `rsync` or `simulated`
- `BKSD_SIMULATION`: Enable simulated hardware adapter
- `BKSD_RPC_ENABLED`: Enable RPC server (default: true)
- `BKSD_RPC_BIND`: RPC bind address (default: 127.0.0.1:9847)
- `BKSD_VERIFY_TRANSFERS`: Verify file integrity after transfer (default: true)

## Dependencies

Key crates:
- `tokio`: Async runtime
- `udev`: Linux device monitoring
- `nix`: Safe wrappers for mount/umount/poll syscalls
- `tokio-rusqlite`: Async SQLite for job persistence
- `serde_json`: JSON serialization for RPC

## Future Vision

- Push notifications: Server pushes job completion events to connected clients
- Subscriptions: `subscribe.jobs` method for real-time event streaming
- Remote daemon support: Run on multiple machines, backup from source device to remote destination
- Additional transfer engines beyond rsync
- Authentication for non-localhost RPC connections
