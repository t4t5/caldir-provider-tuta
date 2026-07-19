# caldir-provider-tuta

Read/write [caldir](https://caldir.org) provider for Tuta calendars.

The provider uses a small vendored and patched copy of Tuta's undocumented Rust SDK. It logs in with a Tuta email and password, stores only the resulting session credentials, and communicates
with caldir through the provider JSON protocol.

## Install

```bash
cargo install --path .
```

`caldir-provider-tuta` should now be available on `PATH` for caldir to discover it.

## Connect

```bash
caldir connect tuta
```

Enter your Tuta email and password. The provider connects to Tuta's hosted service at
`https://app.tuta.com`.

## Sync behavior

- `X-TUTA-ITEM` links a local VEVENT to Tuta's `<list-id>/<element-id>`.
- Reads bypass the SDK's recurrence-expanding facade and load the underlying encrypted entities, preserving recurring masters and RRULEs.
- Tuta exposes no per-event modification time. Pulled events therefore have no `LAST-MODIFIED`; caldir uses its saved sync base and Tuta's `SEQUENCE` as the fallback direction signals.
- Short and long event lists are selected using Tuta's rule: recurring or longer than 15 days means long.
- Moving an event in time, moving calendars, or changing its short/long class deletes and recreates the entity with the same UID. Other edits update in place.
- Recurrence overrides are separate entities and the master receives the matching excluded date.
- Events with attendees are readable, but edits are refused to avoid corrupting invitation state.

## Updating the SDK

The three upstream crates and GPL license are under `vendor/tuta-sdk`. The local changes are recorded in `patches/`:

1. expose resumable credentials after `create_session`;
2. preserve custom IDs on POST;
3. implement entity DELETE;
4. route an SDK diagnostic through logging so it cannot corrupt the provider's JSON stdout protocol.

To update from a Tutanota checkout:

```bash
just vendor /path/to/tutanota
cargo test -p caldir-provider-tuta
```

The script recopies the three crates, reapplies the patch series, and records the new commit.

## Limitations

- The live login and wire recipe require validation with a dedicated account after every SDK bump.
- Reminder offsets are synchronized with the current Tuta user's encrypted alarm list. Tuta supports minute, hour, day, and week offsets before an event start.
- Attendees, organizer details, invitations, birthday calendars, incremental sync, and calendar management are not implemented.
- Floating date-times are stored as UTC wall time because Tuta stores instants rather than RFC 5545 floating times.
- An outdated SDK may be rejected by Tuta when its pinned client version expires.

## License

GPL-3.0-only.
