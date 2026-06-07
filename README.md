# dns-reconciler

`dns-reconciler` is a stateless Rust service that synchronizes active VyOS Kea DHCPv4 leases from the VyOS DHCP lease CSV file to Cloudflare DNS `A` records.

The service is designed for VyOS container use. Domains, Cloudflare settings, credentials, DNS suffixes, and DHCP subnet selection are provided only through environment variables.

## Architecture

```text
VyOS DHCP lease CSV
  -> dns-reconciler
  -> Cloudflare DNS API
```

The service runs a full sync immediately at startup. It also watches the lease CSV file set for changes, applies a short debounce window, and then runs a full sync. A periodic sync remains as a drift recovery path.

## Lease Source

The default lease path is the VyOS DHCP directory:

```text
/config/dhcp
```

The source can be set with:

```env
VYOS_DHCP4_LEASES_PATH=/config/dhcp
```

The service reads every file whose name starts with `dhcp4-leases.csv` from that directory. This covers files such as `dhcp4-leases.csv` and `dhcp4-leases.csv.2`.

The CSV file set is treated as the Source of Truth. Files are read in modification-time order. When the same IPv4 address appears more than once, the later row wins, so a newer inactive row overrides an older active row. The service only uses active rows from selected DHCP subnet IDs. File read or parse errors cause the sync cycle to fail closed. In that case, Cloudflare state is not modified for that cycle.

Expected CSV columns include:

```text
address,valid_lifetime,expire,subnet_id,hostname,state
```

Additional columns are allowed.

## Change Watch And Debounce

The service detects lease file set metadata changes and schedules a full sync after a debounce window.

Optional settings:

```env
LEASE_FILE_WATCH_ENABLED=true
LEASE_FILE_WATCH_INTERVAL_MILLIS=250
LEASE_FILE_DEBOUNCE_MILLIS=500
```

The file event is only a trigger. DNS desired state is always rebuilt from the complete lease file set.

## DHCP_SUBNET_IDS

`DHCP_SUBNET_IDS` selects the Kea subnet IDs that this service manages.

```env
DHCP_SUBNET_IDS=10
DHCP_SUBNET_IDS=10,20,30
```

Rules:

- Only leases with a selected `subnet_id` are considered.
- Leases without `subnet_id` are skipped.
- An empty or invalid `DHCP_SUBNET_IDS` value fails startup.

## Environment Variables

Required:

```env
VYOS_DHCP4_LEASES_PATH=/config/dhcp
DHCP_SUBNET_IDS=10
DNS_ZONE=example.com.
MANAGED_RECORD_SUFFIX=dhcp.example.com.
CLOUDFLARE_ZONE_ID=replace-me
CLOUDFLARE_API_TOKEN=replace-me
DEFAULT_TTL=300
SYNC_INTERVAL_SECONDS=300
LOG_LEVEL=info
```

Optional:

```env
DRY_RUN=false
CLOUDFLARE_API_BASE_URL=https://api.cloudflare.com/client/v4
CLOUDFLARE_REQUEST_TIMEOUT_SECONDS=30
LEASE_FILE_WATCH_ENABLED=true
LEASE_FILE_WATCH_INTERVAL_MILLIS=250
LEASE_FILE_DEBOUNCE_MILLIS=500
```

`MANAGED_RECORD_SUFFIX` must be below `DNS_ZONE`. For example, `dhcp.example.com.` is valid for `example.com.`.

## DNS Rules

The final DNS name is:

```text
<hostname>.<MANAGED_RECORD_SUFFIX>
```

Example:

```text
host01 -> host01.dhcp.example.com
```

Managed records:

- Type: `A`
- TTL: `DEFAULT_TTL`
- Proxied: `false`

Validation rules:

- Only records inside `DNS_ZONE` are managed.
- Only records below `MANAGED_RECORD_SUFFIX` are managed.
- The zone apex is never managed.
- Wildcards are not allowed.
- Hostnames must be a single valid DNS label.
- Empty hostnames are skipped.
- Hostnames are lowercased and trailing dots are removed.
- Leases that are inactive or expired are skipped.
- If the same hostname appears more than once, the lease with the newest ordering timestamp is selected. Ties are resolved deterministically by IPv4 address.

## Cloudflare Token Permissions

Use a Cloudflare API Token scoped to the selected zone only.

Recommended permission:

- Zone / DNS / Edit

Zone resource:

- Include / Specific zone / `example.com`

The API token is never printed by the service.

## Build

```sh
cargo build --release
```

## Test

```sh
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

## Docker

Build:

```sh
docker build -t dns-reconciler:local .
```

Run:

```sh
docker run --rm --read-only --network=host \
  --mount type=bind,source=/config/dhcp,target=/config/dhcp,readonly \
  -e VYOS_DHCP4_LEASES_PATH=/config/dhcp \
  -e DHCP_SUBNET_IDS=10 \
  -e DNS_ZONE=example.com. \
  -e MANAGED_RECORD_SUFFIX=dhcp.example.com. \
  -e CLOUDFLARE_ZONE_ID=replace-me \
  -e CLOUDFLARE_API_TOKEN=replace-me \
  -e DEFAULT_TTL=300 \
  -e SYNC_INTERVAL_SECONDS=300 \
  -e LOG_LEVEL=info \
  dns-reconciler:local
```

## VyOS Example

Adapt names and image references for your environment. The domain values below are examples only.

```text
set container name dhcp-dns-sync image 'ghcr.io/example/dns-reconciler:latest'
set container name dhcp-dns-sync allow-host-networks
set container name dhcp-dns-sync uid '101'
set container name dhcp-dns-sync gid '109'
set container name dhcp-dns-sync volume dhcp-leases source '/config/dhcp'
set container name dhcp-dns-sync volume dhcp-leases destination '/config/dhcp'
set container name dhcp-dns-sync volume dhcp-leases mode 'ro'
set container name dhcp-dns-sync environment VYOS_DHCP4_LEASES_PATH value '/config/dhcp'
set container name dhcp-dns-sync environment DHCP_SUBNET_IDS value '10'
set container name dhcp-dns-sync environment DNS_ZONE value 'example.com.'
set container name dhcp-dns-sync environment MANAGED_RECORD_SUFFIX value 'dhcp.example.com.'
set container name dhcp-dns-sync environment CLOUDFLARE_ZONE_ID value 'replace-me'
set container name dhcp-dns-sync environment CLOUDFLARE_API_TOKEN value 'replace-me'
set container name dhcp-dns-sync environment DEFAULT_TTL value '300'
set container name dhcp-dns-sync environment SYNC_INTERVAL_SECONDS value '300'
set container name dhcp-dns-sync environment LEASE_FILE_WATCH_ENABLED value 'true'
set container name dhcp-dns-sync environment LEASE_FILE_WATCH_INTERVAL_MILLIS value '250'
set container name dhcp-dns-sync environment LEASE_FILE_DEBOUNCE_MILLIS value '500'
set container name dhcp-dns-sync environment LOG_LEVEL value 'info'
commit
save
```

The `uid` and `gid` values must allow the container process to read the lease directory and files. On many VyOS systems, the lease files are owned by `_kea`, so matching that UID/GID is useful.

## Dry Run

Dry run computes the same plan and logs planned actions without calling Cloudflare mutation endpoints.

```sh
docker run --rm --read-only --network=host \
  --mount type=bind,source=/config/dhcp,target=/config/dhcp,readonly \
  -e DRY_RUN=true \
  -e VYOS_DHCP4_LEASES_PATH=/config/dhcp \
  -e DHCP_SUBNET_IDS=10 \
  -e DNS_ZONE=example.com. \
  -e MANAGED_RECORD_SUFFIX=dhcp.example.com. \
  -e CLOUDFLARE_ZONE_ID=replace-me \
  -e CLOUDFLARE_API_TOKEN=replace-me \
  -e DEFAULT_TTL=300 \
  -e SYNC_INTERVAL_SECONDS=300 \
  -e LOG_LEVEL=info \
  ghcr.io/example/dns-reconciler:latest
```

## Logs

Logs are JSON structured events emitted with `tracing`.

Key events:

- `startup`
- `configuration_loaded`
- `lease_file_watch_started`
- `lease_file_changed`
- `lease_file_event_received`
- `lease_file_read_started`
- `lease_file_set_loaded`
- `lease_file_read_completed`
- `sync_started`
- `sync_completed`
- `record_create`
- `record_update`
- `record_delete`
- `record_skipped`
- `error`

Each `sync_completed` event includes:

- `leases_total`
- `leases_selected`
- `records_created`
- `records_updated`
- `records_deleted`
- `records_unchanged`
- `records_failed`

Example:

```sh
show log container dhcp-dns-sync
```

## Troubleshooting

Lease file errors:

- Confirm that `/config/dhcp` exists on the host.
- Confirm that the directory is mounted read-only into the container.
- Confirm that the container UID/GID can read the lease files.
- Confirm that the file contains the expected CSV columns.

No records are created:

- Confirm `DHCP_SUBNET_IDS` matches lease `subnet_id` values.
- Confirm leases have valid non-empty hostnames.
- Confirm leases are active and not expired.
- Run with `DRY_RUN=true` and `LOG_LEVEL=debug` to inspect the plan.

Cloudflare errors:

- Confirm `CLOUDFLARE_ZONE_ID` references the same zone as `DNS_ZONE`.
- Confirm the token has DNS Edit permission for the selected zone.
- Confirm `MANAGED_RECORD_SUFFIX` is below `DNS_ZONE`.

Safety behavior:

- If lease file read or parse fails, Cloudflare state is not modified.
- Records outside `MANAGED_RECORD_SUFFIX` are left unchanged.
- Only `A` records are managed.
