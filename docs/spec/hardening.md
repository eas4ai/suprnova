# Hardening

Status: Draft
Prefix: HRD

Drafted 2026-10-11 from the Daybreak defensive audit of 2026-10-11
(`.planning/daybreak-audit.md`, six findings confirmed from source, two
High), each verified against the code by the controller the same day, and
the developer's ruling of 19:38 ("confirmed", to the proposal that the six
findings become a defect round of their own after the generators and the
Live kit, before the 4.0.0 tag). The Observed section describes framework
`2fffe64fc`.

## Observed at 2fffe64fc

Status: Observed

- `collect_after` awaits every body frame with no per-frame or total
  deadline (`framework/src/http/body.rs:83-136`); the server's one timeout
  covers the request head (`SERVER_HEADER_READ_TIMEOUT`,
  `framework/src/server.rs:91-108`) and `SERVER_MAX_CONNECTIONS` defaults to
  none. The dogfood application installs `TimeoutMiddleware::default()`
  (`app/src/bootstrap.rs:359`); neither generated bootstrap does
  (`suprnova-cli/src/templates/files/backend/bootstrap.rs.tpl`,
  `files/api/src/bootstrap.rs.tpl`).
- `MagickCliDriver::guard` returns `None` for bytes the sniffer does not
  name (`framework/src/media/magick.rs:137-147`), and `input_spec(None)`
  passes bare stdin (`magick.rs:1231-1236`), so ImageMagick chooses the
  coder from the bytes; MVG and MSL are scripts.
- The Web Push strict policy requires HTTPS, refuses IP literals and a list
  of metadata names, and documents that it resolves no host name
  (`crates/suprnova-web-push/src/client.rs:422-427`); a `PushServiceRejected`
  keeps up to 8 KiB of the remote body.
- `app/src/controllers/auth_reset.rs:80` calls `PasswordReset::complete`,
  which discards the two revocation outcomes
  (`framework/src/auth_flows/password_reset.rs:242-246`), and redirects to
  the success page whenever the password write succeeded; the scaffold's
  controller uses `complete_with_outcome`.
- `StaticFiles` canonicalizes and then opens by path name
  (`framework/src/static_files.rs:69-95`); `LiveUiAssets` canonicalizes,
  then calls `metadata` and `read` by path name
  (`framework/src/live/ui_assets.rs:161-176`). `framework/src/filesystem/confined.rs`
  resolves against held directory descriptors with `O_NOFOLLOW`.
- `OriginPolicy::SameOrigin` compares the `Origin` host and an effective
  port derived from the `Origin`'s own scheme against `Host`, and documents
  that the scheme is not compared (`framework/src/ws/mod.rs:68-75`);
  `Request::scheme` is trusted-proxy aware (`framework/src/http/request.rs:960-1011`).

## Requirements

[HRD-001] The server MUST bound how long a request body takes to arrive:
a body whose next frame does not arrive within a progress deadline, or
whose whole takes longer than a total deadline, MUST be abandoned and its
connection closed, with finite production defaults that the environment
can change; and the full-stack and API bootstraps `suprnova new` writes
MUST install `TimeoutMiddleware::default()`.
Falsifier: a client that completes the headers and then trickles an allowed-size body one byte at a time keeps the connection open past the deadlines; a fresh scaffold's bootstrap installs no timeout middleware.
Mechanism: `hardening`.
Rationale: Daybreak DBK-01 (High); `TimeoutMiddleware` bounds the pipeline but not the read the server performs before it, and the generated applications had neither.
Status: Agreed 2026-10-11

[HRD-002] The Magick driver MUST hand ImageMagick a coder-qualified input
(`<coder>:-`) for every format the framework recognises and MUST refuse
input bytes it cannot name, so that ImageMagick never selects a coder from
the bytes of an upload; autodetection of unrecognised bytes MUST be
unavailable unless an application opts into it by name.
Falsifier: bytes the sniffer does not recognise reach ImageMagick on bare stdin; an MVG or MSL document, or a URL pseudo-coder input, is handed to ImageMagick under the default configuration.
Mechanism: `hardening`.
Rationale: Daybreak DBK-02 (High, conditional on `IMAGE_DRIVER=magick`); the host's ImageMagick policy was the only barrier.
Status: Agreed 2026-10-11

[HRD-003] The Web Push client's strict endpoint policy MUST resolve the
endpoint's host name and refuse a destination whose address is loopback,
private, link-local, multicast, unspecified, reserved or a known metadata
address; it MUST connect to the address it validated while keeping the
host name for TLS and the `Host` header; and an error that reaches the
caller MUST NOT carry the remote response body.
Falsifier: an endpoint whose host name resolves to `127.0.0.1` or to `10.0.0.0/8` is sent to; the connection resolves the name a second time; a `PushServiceRejected` or the framework channel's error carries the remote body.
Mechanism: `hardening`.
Rationale: Daybreak DBK-03 (Medium); the stored endpoint is user-derived, so the policy is the boundary.
Status: Agreed 2026-10-11

[HRD-004] The dogfood application's password reset MUST report success
only when the password write and both revocations (the user's other
sessions and their remember tokens) succeeded; a revocation failure MUST
answer an error response and be logged with the user's id, never the
success redirect.
Falsifier: with a session store whose revoke fails, completing a reset answers `302 /?reset=ok`.
Mechanism: `hardening`.
Rationale: Daybreak DBK-04 (Medium); `complete_with_outcome` reports the outcomes and the scaffold's controller already reads them.
Status: Agreed 2026-10-11

[HRD-005] `StaticFiles` and the Live UI asset server MUST resolve a
requested path relative to a held directory descriptor without following
symlinks, as the confined filesystem does, and MUST perform the type and
size checks on the handle they then read from.
Falsifier: a directory under the served root replaced by a symlink after the path check serves a file outside the root; the size bound is checked on one handle and the bytes read from another.
Mechanism: `hardening`.
Rationale: Daybreak DBK-05 (Medium, needs local write access to the served tree); the confined filesystem states that a path check followed by a path-name syscall is racy.
Status: Agreed 2026-10-11

[HRD-006] `OriginPolicy::SameOrigin` MUST compare the complete origin:
the request's public scheme (`Request::scheme`, after trusted-proxy
processing), its validated host and the effective port, and MUST refuse
the upgrade when the public scheme cannot be determined.
Falsifier: `Origin: http://victim.example` opens a WebSocket on a request whose public scheme is `https`; a request whose public scheme is unknown admits an origin by host and port alone.
Mechanism: `hardening`.
Rationale: Daybreak DBK-06 (Medium, conditional); Live's transport already compares the exact origin, general routes under the default policy did not.
Status: Agreed 2026-10-11

[HRD-007] Every sign-in the framework issues through a provider MUST record
the user's authentication epoch: the value `Authenticatable::auth_epoch`
returns, or, when the model exposes none, a digest of the password hash
`get_auth_password` returns. The session MUST carry the epoch of each
guard's sign-in and every remember credential MUST be bound to the epoch at
issue (stored with it, or mixed into the hash its verifier is checked
against). On every request that resolves a signed-in user from the session
or from a remember credential the framework MUST compare the recorded epoch
with the user's current one and, on a mismatch, end that sign-in (the
guard's identity leaves the session, the remember row is deleted) and
answer as for a guest, before the user reaches the application. A user
whose provider exposes neither an epoch nor a password hash records no
epoch and is not checked.
Falsifier: a session signed in before a password change answers an authenticated route `200` after the change although the eager revocation failed; a remember cookie issued before the change signs the user in after it.
Mechanism: `hardening`.
Rationale: Daybreak DBK-04's first remedy, ruled in by the developer on 2026-10-12 ("I want all defects identified by Daybreak to be remediaited for 4.0.0"); eager revocation alone (HRD-004) leaves a stolen session usable when the store refuses, since nothing re-checks a session against the password on later requests. The digest of the password hash advances with every password write, so no table changes.
Status: Agreed 2026-10-12

[HRD-008] When a password reset's revocation of the user's other sessions
or remember credentials fails, the framework MUST have queued an
idempotent job that retries both revocations until both stores confirm
them, before the reset answers; the dogfood application's reset MUST queue
it (with the error HRD-004 requires) and a framework job type MUST exist
for applications to queue themselves.
Falsifier: with a session store whose revoke fails, completing a reset queues no revocation job; or the job reports done while the store still holds the user's sessions.
Mechanism: `hardening`.
Rationale: Daybreak DBK-04's third remedy (the developer's ruling of 2026-10-12): an error after the password has changed is not enough on its own, since the reset token is already consumed; the retry makes the revocation durable while HRD-007 keeps the stale credentials from working meanwhile.
Status: Agreed 2026-10-12

[HRD-009] The server MUST cap concurrent connections finitely by default:
with `SERVER_MAX_CONNECTIONS` unset or blank the cap is
`DEFAULT_MAX_CONNECTIONS` (10000); zero or an unparseable value keeps the
finite misconfiguration fallback; an explicit positive value sets the cap.
Falsifier: with `SERVER_MAX_CONNECTIONS` unset, `ServerConfig::from_env` reports no cap, or the server accepts the ten-thousand-and-first concurrent connection without a queue.
Mechanism: `hardening`.
Rationale: Daybreak DBK-01's third remedy (the developer's ruling of 2026-10-12); the deadlines of HRD-001 bound each connection in time, the cap bounds their number, and an unbounded default let a slow client population exhaust file descriptors before any deadline fired.
Status: Agreed 2026-10-12

[HRD-010] The Magick driver MUST run ImageMagick under a framework-owned
policy whether or not autodetection is on: a `policy.xml` the framework
writes and verifies before each run, reached through
`MAGICK_CONFIGURE_PATH`, that denies the URL, HTTP, HTTPS, FTP, MVG, MSL,
TEXT, LABEL, CAPTION, PANGO, SCREENSHOT, X, EPHEMERAL, PS, PS2, PS3, EPS,
PDF and XPS coders, every delegate, and `@file` path reads; a run whose
policy cannot be written or verified MUST refuse before ImageMagick starts.
Falsifier: the child process starts with `MAGICK_CONFIGURE_PATH` unset or naming a directory whose `policy.xml` differs from the framework's; or an MVG, MSL or `url:` input under `IMAGE_MAGICK_AUTODETECT` reaches a coder the policy names.
Mechanism: `hardening`.
Rationale: Daybreak DBK-02's third remedy (the developer's ruling of 2026-10-12): the opt-in exists (HRD-002) but ran under the host's policy; ImageMagick merges every `policy.xml` on its configure path and a policy only removes rights, so the framework's tightens whatever the host allows.
Status: Agreed 2026-10-12

[HRD-011] `EndpointPolicy` MUST offer an allowlist: a policy that admits a
subscription endpoint only when its origin (scheme, host and port) is one
of the configured push-service origins, after `Strict`'s checks; the
framework's Web Push channel MUST use it when `WEB_PUSH_ENDPOINT_ORIGINS`
names one or more origins, and the manual MUST list the known browser
push services' origins.
Falsifier: with the allowlist set to one origin, an endpoint at another public host is sent to.
Mechanism: `hardening`.
Rationale: Daybreak DBK-03's fourth remedy (the developer's ruling of 2026-10-12): the strict checks of HRD-003 refuse private destinations; the allowlist refuses every destination that is not a push service, for deployments that can name theirs.
Status: Agreed 2026-10-12
