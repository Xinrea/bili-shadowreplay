# Recording storage

`RecordingStore` selects a backend and scopes it to a recording. Persistent
object, log, media-repair, range-read, listing and deletion operations go through
`StorageBackend`; only `LocalBackend` implements persistent filesystem I/O.
Backend selection does not require a persistent local replica.

## S3-compatible storage

`S3Backend::new(&S3Config, &S3Credentials)` uses the `object_store` S3 client for AWS SigV4,
HTTP/TLS, retries and service pagination. Configure a pre-existing bucket,
region, optional HTTP(S) endpoint, optional object prefix and path-style access.
Only general-purpose buckets with lexically ordered ListObjectsV2 are supported.
Credentials are supplied explicitly by the caller; the backend never reads
environment credentials or AWS profiles. The application accepts credentials in
its storage settings, persists them separately using its credential cipher, and
never returns them through the configuration API. `S3Credentials` redacts Debug
output. Clients hold a credential snapshot; recreate them after rotation.

Custom endpoints are service origins. Path-style uses `endpoint/bucket/key`;
virtual-hosted access derives `bucket.endpoint/key`. An already bucket-qualified
DNS endpoint is accepted without adding the bucket twice. IP endpoints require
path-style; unqualified bucket-specific CNAMEs are not supported. Empty endpoints
retain the SDK's AWS defaults. Media and catalog clients share this conversion
without rewriting persisted endpoint identities.

Bounded GETs check both response metadata and streamed chunks. Range reads use
HEAD plus a conditional ranged GET to retain EOF semantics without downloading
whole segments or racing object replacement. Listings include the exact prefix
object and its descendants, never similarly named siblings. PUT replaces one
complete object (no multipart uploads; respect the service's single-PUT limit).
The existing chunked logs, materialization and scoped deletion work unchanged.
Do not use overlapping prefixes for independent backends.

Normal tests include a local HTTP protocol fixture. For the opt-in RustFS test
and local Docker Compose setup, see
[`docs/development/backend/recording-storage.md`](../../../docs/development/backend/recording-storage.md#用-rustfs-本地验证).

## Backend contract

- `write` atomically replaces one object and acknowledges only after it is
  readable. Multi-object updates are not transactions.
- `read_bounded` stops at the limit rather than downloading an entire oversized
  object. `read_range` reads only an exclusive-end byte range and may return a
  short/empty result at EOF; reversed ranges are invalid.
- `list(prefix, cursor, limit)` lists a **subtree**, not a raw string prefix:
  `room1` matches `room1/file`, not `room10/file`. Empty prefix lists the
  namespace root. Keys are lexical, pages are bounded, cursors are opaque.
  A backend whose native API uses raw prefixes must supply the `/` delimiter.
  Do not mutate the namespace while traversing pages.
- `remove_all` restarts listing after each removal batch. It is idempotent and
  retryable, but a failure can follow successful removals. Stop writers before
  deleting a recording. Filesystem directories may remain empty afterward.
- Local paths reject symlinks encountered beneath the configured root. Local
  roots must be application-controlled; this is not a sandbox against another
  process replacing paths between validation and I/O.

## Durable logs

Use `open_log`, `AppendLog::append`, `read_log` and `log_exists`, not object `read`
or `exists`, for event/ad logs. `read_log` includes both an old plain base object
and any committed chunks. Missing logs return `NotFound`; malformed metadata,
missing committed chunks and backend failures propagate as errors.

The default backend implementation needs only object operations:

1. Read an eight-byte committed chunk count.
2. Write the append's bytes in chunks of at most 64 KiB.
3. Atomically replace the count to commit all of those chunks.

One append costs one bounded metadata GET, `ceil(bytes / 64 KiB)` chunk PUTs and
one eight-byte metadata PUT. It never rewrites earlier log data or an unbounded
index. Empty appends write nothing. Opening an absent log writes an empty count;
opening an existing log reads the count. Reading costs one legacy-base GET, one
metadata GET and one GET per committed chunk; the returned `Vec` necessarily
holds the whole log.

Chunks live under the reserved `.recording-logs` directory **alongside** the log,
with a collision-free encoded filename. Copy this directory when moving archives.
Uncommitted chunks after failed writes are ignored and reused on a retry; recording
deletion also removes orphan chunks. An acknowledgement loss may mean the append
committed; automatic exactly-once retries are not promised.

There is one writer per recording/log. Clones of the same append handle serialize
writes. A cancelled append finishes in the background while retaining that handle's
lock; `flush` waits behind it. Separately opened handles must not overlap writes.
Every successful append is already durable; flush/drop is not needed to publish
buffered data. `LocalBackend` optimizes plain logs using append + file sync, and
continues the chunk format when reading an archive copied from an object backend.

## Path-based consumers

`materialize(&keys)` copies only explicitly requested objects into a `LocalAssets`
lease. Keep the lease alive while FFmpeg or another consumer uses its paths.
`LocalAssets::new()` creates an empty lease for scratch-only tasks. Lease writes
never publish archive objects; callers explicitly publish any desired results.
Failure, cancellation and drop clean up temporary files, including queued blocking
writes. There is no implicit media-dependency expansion or permanent local cache.

`LocalWorkspace`, `workspace()`, `local_root()`, and path-taking constructors remain
explicit legacy adapters. Do not use them to discover/read persistent assets for
a recorder with an arbitrary backend.

## Journal-driven object transfer

`ObjectTransfer` scopes a source and destination `RecordingStorage` to a single
archive. Persist its `ObjectTransferManifest` and a fresh token before `copy`;
commit the application's archive binding before `cleanup_source`; persist the
terminal phase before `finish`. Local-to-local rename/copy stays on `LocalTransfer`.

The object manifest pins endpoint/root identity, namespace, every key's length,
SHA-256 and source revision. `.recording-transfer.json` is a reserved destination
ownership marker. Unowned nonempty targets are rejected, including identical
files. Retry reuses verified objects and never overwrites a conflicting key.
Cleanup verifies the complete target and remaining source before any removal.
Finalizing retries only remove the marker, not recreated source data.

The backend's `create`, `create_from_file`, `download_to_file` and
`transfer_revision` methods are opt-in transfer capabilities; default implementations
return `Unsupported`. Local publication uses a synced no-clobber tempfile. S3
publication checks target absence, then uses an SDK-signed ordinary streaming PUT
under the application's single-writer transfer gate, with a 5 GiB single-PUT
limit. This is not distributed no-clobber. No multipart or server-side copy is required. A private system-temp
directory holds one object at a time; ensure it has sufficient free space.
S3 revisions use ETags/version IDs; local revisions use filesystem identity and
timestamps. Stop independent external writers: portable S3 deletion cannot be
made atomic with a prior multi-object validation.

## Journal-driven local transfer

`LocalTransfer` is a synchronous local-filesystem primitive. Call it through
`spawn_blocking`; it does not select a backend or implement S3. All persistent
transfer filesystem access stays inside this crate.

### Fast same-filesystem rename

```text
LocalTransfer::new(source_root: &Path, target_root: &Path) -> io::Result<Self>
same_filesystem(&self) -> bool
prepare_rename(&self, namespace: &str) -> io::Result<RenameManifest>
check_rename(&self, namespace: &str, manifest: &RenameManifest) -> io::Result<()>
rename(&self, namespace: &str, manifest: &RenameManifest) -> io::Result<()>
finish_rename(&self, namespace: &str, manifest: &RenameManifest) -> io::Result<()>
is_cross_device_error(error: &io::Error) -> bool
```

`RenameManifest` is exported and `Clone + Debug + PartialEq + Eq + Serialize +
Deserialize`. Its private serialized fields bind the namespace, both canonical
root paths and their stable filesystem identities, and the original namespace
directory identity. Keep the exact serialized manifest across restart; do not
rebuild it to authorize recovery after an identity conflict.

Preparation and recovery are **O(1) in archive size**, with O(path depth) bounded
metadata operations and memory. They never list, read, hash, or copy payload
files, and do not require free space equal to the archive size. The entire
directory is moved as opaque payload, including hidden logs, hardlinks, special
files and symlinks **inside** it; those symlinks are not followed or inspected.
This differs intentionally from the stricter copy manifest below. Namespace
paths, roots and traversed ancestors must themselves be symlink-free directories.
Root/namespace ancestor aliases and overlapping roots are rejected.

The application must stop and flush archive writers, serialize operations for
the archive, and retain its reservation throughout recovery. This primitive
does not flush existing payload bytes or protect against hostile concurrent
path replacement. The application DB journal is the ownership proof; there is
no transfer token, proof file, payload scan, or source-deletion step:

1. Use `same_filesystem` as a root device/volume hint, then `prepare_rename` for
   read-only preflight. Nested mounts can still cause rename to return EXDEV.
2. **Durably journal the exact `RenameManifest` before calling `rename`.**
3. `rename` atomically moves the namespace without replacing any existing target,
   even an empty directory. It then syncs both source and destination parents,
   including destination ancestors created for nested namespaces.
4. Durably commit the destination binding in the DB while retaining the journal.
5. Call `finish_rename`, then delete the journal and release the reservation.

`check_rename` is read-only and accepts only the exact original source with
absent target, or absent source with the exact moved directory at the target.
An existing reserved copy ownership proof for the namespace is also a conflict,
even when its staged copy is unpublished; rename cannot overtake a live copy
journal. Finish or reconcile that copy protocol before preparing a new rename.
It checks **both** persisted root identities even after a new `LocalTransfer`
is constructed. Missing/recreated destinations and recreated sources conflict;
identical bytes or an empty recreated directory do not grant ownership.
`rename` retries recognize the moved directory and repeat durability barriers.
`finish_rename` requires that moved state, repeats the barriers, and **never
deletes source or target**. A crash after rename but before DB commit is therefore
recoverable using the original journal without hashing payloads.

Only `is_cross_device_error` permits a copy fallback: it recognizes raw Unix
`EXDEV` or Windows `ERROR_NOT_SAME_DEVICE` (17), according to the current platform.
An error kind or message alone is not enough. Permission, identity conflict,
unsupported atomic rename, and durability failures must propagate; never rebuild
a copy manifest merely because fast rename failed. Sync failures can occur
**after** the move; retain the journal and retry its rename protocol.

Linux/macOS use atomic no-replace rename and directory fsync. Windows uses
write-through no-replace rename; portable directory fsync is unavailable there.
Other platforms fail closed if atomic no-replace rename is unsupported. Once a
journal is finalized, a fresh transfer in the opposite direction needs only a
fresh manifest; there is no proof-file reclamation to block round trips.

### Copy-and-cleanup protocol

Copy APIs use a persistent `TransferManifest` for integrity and recovery:

```text
LocalTransfer::new(source_root: &Path, target_root: &Path) -> io::Result<Self>
available_bytes(&self) -> io::Result<u64>
copy_available_bytes(&self, namespace: &str) -> io::Result<u64>
inspect(&self, namespace: &str) -> io::Result<TransferManifest>
check_destination(&self, namespace: &str, token: &str, manifest: &TransferManifest) -> io::Result<()>
required_bytes(&self, namespace: &str, token: &str, manifest: &TransferManifest) -> io::Result<u64>
copy(&self, namespace: &str, token: &str, manifest: &TransferManifest) -> io::Result<()>
cleanup_source(&self, namespace: &str, token: &str, manifest: &TransferManifest) -> io::Result<()>
finish(&self, namespace: &str, token: &str) -> io::Result<()>
```

Both roots must already exist. Roots and all traversed ancestors must be
symlink-free, canonicalizable directories, and must not alias, equal or contain
each other in either direction. Root identity is rechecked on every operation.
Namespaces may be nested, e.g. `bilibili/123/live`, and use normal storage-key
validation. The `.recording-transfer-` component prefix is reserved.

For copy preflight, use `copy_available_bytes(namespace)`, not the
root-only `available_bytes()`. It queries the nearest existing destination
namespace **parent** ancestor without creating directories, so it reports the
correct filesystem for a nested mounted destination. Per-chunk capacity checks
query the actual owned stage, not the configured target root.

New copies always create staging as a private sibling of the destination
namespace, even when the target root and namespace parent report the same device.
Equal device IDs do not prove that rename can cross a mount boundary: Linux bind
mounts can still return EXDEV. Adjacent staging makes atomic no-replace
publication possible after an EXDEV rename fallback without mount detection.
The ownership proof stays at the target root and uses version 2 with a strictly
validated root-relative staging path. Only the exact namespace parent is allowed
for version 2; absolute paths, traversal, other parents and symlink ancestors are
rejected. All staging parents are synced before proof publication.

Only this proof format is supported; retries use the recorded stage rather than
reselecting it. Never move mounts during a live transfer. Publication fails
closed if its mount boundary changes; staging is not silently relocated or
re-owned. Finalization removes only the verified stage and
syncs its actual parent, leaving the namespace and shared parent assets intact.

`TransferManifest` is `Clone + Serialize + Deserialize`; `total_bytes()` returns
the checked sum of all file sizes. Public file entries have relative `path`,
`size` and `sha256: [u8; 32]`; directory entries include empty directories.
Serialized private identity metadata binds the snapshot to its original source.
Ownership proof JSON is limited to 64 MiB; an oversized manifest fails before
proof publication rather than creating a transfer that cannot be resumed.
Copy includes every regular file below the namespace, including hidden logs and
checkpoints, but not shared files in its parents (such as a legacy room cover).
Symlinks, special files, nonportable filenames, directory aliases and multiple
archive paths hardlinked to the same file are rejected, never silently skipped.

The application must stop archive writers/readers as appropriate and serialize
journal operations for that archive. Roots must be application-controlled: this
API rejects observed symlinks and changed objects, but does not sandbox malicious
concurrent path replacement by other processes.

Use a fresh UUID token and retain the exact manifest throughout this sequence:

1. Inspect and run read-only destination/space preflight.
2. **Durably store manifest + token in the application DB journal before copy.**
3. Copy. Payload I/O uses a fixed 64 KiB buffer; available space is checked before
   each chunk. Memory for the manifest scales with the number of entries.
   Files are hashed, flushed and installed in a private owned stage, then the
   complete archive is verified and published with atomic no-replace rename.
4. Durably commit the destination binding in the app DB, retaining the journal.
5. Cleanup source. Every attempt verifies the owned destination's entire tree
   and SHA-256 hashes and repeats publication durability barriers. Then it
   verifies **all remaining** source entries before unlinking any. Byte equality
   alone is insufficient: original filesystem identity and timestamps must also
   match. Missing entries allow interrupted-cleanup retry; recreated, modified
   or new entries stop cleanup. Only verified files and empty directories are
   removed; source cleanup never uses recursive deletion.
6. **Durably mark the journal as terminal/finalizing**, after successful source
   cleanup. This phase must never repeat copy or source cleanup. Retain the
   archive reservation and call `finish` to remove ownership/staging metadata;
   a missing proof is an idempotent success. Only after `finish` succeeds may the
   application delete the journal and release the archive reservation. This
   prevents an orphan proof from blocking later transfers back to this backend.

On failure, retain the journal and retry using the same token/manifest. A
published owned destination needs zero additional payload bytes; a partial stage
conservatively reports the full manifest size. A different token, unowned target
(including an empty directory or identical files), replaced stage, unexpected
archive entries or mismatching hashes fails closed without replacing the target.
An exclusive proof-file lock rejects concurrent cooperating mutations.

Ownership proof is immutable and outside the published archive. Crashes before
proof installation or during terminal reclamation may leave harmless private
orphan staging directories; they are not inferred to be owned from names alone.
Do not delete a live proof or reuse a token. If the source/destination genuinely
changed, stop automatic retry and reconcile the journal rather than rebuilding
its manifest to authorize deletion.

Linux/macOS use no-replace rename and fsync file/directory durability barriers,
including on recovery. Windows uses file flushing and write-through no-replace
publication; portable directory fsync is unavailable there. Other OSes fail
closed when atomic no-replace rename is unavailable.
