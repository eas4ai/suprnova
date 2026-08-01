-- Deterministic bulk load for the benchmark dataset.
--
-- Generated inside Postgres from generate_series rather than streamed in
-- from a file. That is faster by an order of magnitude, needs no 100 GB
-- intermediate artifact, and — because every value is derived from the
-- series index by a pure function — produces byte-identical data on the
-- Suprnova database and the Laravel database by construction. The parity
-- contract's "same rows on both sides" becomes a property of the design
-- instead of something a later assertion has to catch.
--
-- No random(). Every pseudo-random value comes from md5 of the row index,
-- so a re-run reproduces the same dataset exactly.
--
-- # Three traps this file deliberately avoids
--
-- ## int4 overflow in the hash arithmetic
--
-- generate_series(1, 200000000) yields *integer*, not bigint — both bounds
-- fit in int4, so that is the overload Postgres picks. Every `i * k` used
-- as a hash input is therefore int4 arithmetic, and int4 tops out at
-- 2147483647. At 200M comments, `i * 11` overflows at i = 195,225,787 and
-- the load dies 97% of the way through the largest table in the set.
--
-- Every multiplication below casts the index to bigint first. The cast is
-- on the index rather than the product because the product is what
-- overflows — `(i * 11)::bigint` would compute the int4 product, overflow,
-- and only then widen the wreckage.
--
-- Note what does *not* catch this: the USERS=1000 smoke test. At 1/1000
-- scale the largest product is 2.2M, four orders of magnitude clear of the
-- limit. This failure mode is invisible below roughly USERS=976,129 and
-- certain above it, which is the worst shape a bug can have in a loader
-- whose full run is the expensive one.
--
-- ## Compressible bodies
--
-- The obvious way to make a variable-length body is repeat('x', n). Do not.
-- Postgres compresses values past the TOAST threshold, and a run of one
-- character compresses to almost nothing — the posts table would land at a
-- fraction of its intended size, sit entirely in cache, and every read
-- benchmark against it would be measuring a table that does not exist at
-- the scale it claims.
--
-- Bodies are therefore capped below the TOAST threshold (~2 KB) so they
-- stay inline in the heap, uncompressed and un-indirected. Heap size then
-- follows directly from row count times average length, which is the
-- property the whole data-scale argument rests on.
--
-- ## Indexes during load
--
-- None are present here. Building them incrementally across 400M inserts
-- costs the better part of a day; building them afterwards with a large
-- maintenance_work_mem is far cheaper. indexes.sql runs second.
--
-- Which is why the drops below exist. TRUNCATE keeps indexes — it empties
-- the table, it does not undo the schema — so on a *second* run the five
-- indexes indexes.sql created last time are still there, and the whole
-- load maintains them row by row. Nothing fails and nothing warns: the
-- data is correct, the load is just far slower than the design, and
-- indexes.sql then reports `already exists, skipping` and finishes in
-- seconds, which reads like the build was fast rather than absent.
--
-- That is what happened on the first full run of this file, seeded by an
-- earlier USERS=1000 smoke test against the same database.
--
-- Only the indexes indexes.sql owns are dropped. The migration's own
-- indexes and constraints stay — taggables' unique index in particular,
-- which the ON CONFLICT clause below depends on.

\set ON_ERROR_STOP on

-- Durability is not wanted for seed data — a crash mid-load is answered by
-- re-running the load, not by recovering it. This is reset at the end;
-- leaving it off would flatter every write scenario in the benchmark.
SET synchronous_commit = off;

\echo '==> dropping bench indexes (rebuilt by indexes.sql after the load)'
DROP INDEX IF EXISTS idx_posts_author;
DROP INDEX IF EXISTS idx_posts_public_id;
DROP INDEX IF EXISTS idx_posts_recent;
DROP INDEX IF EXISTS idx_taggables_reverse;
DROP INDEX IF EXISTS idx_users_email;

\echo '==> truncating'
TRUNCATE users, posts, comments, profiles, tags, taggables, roles, role_user
    RESTART IDENTITY CASCADE;

-- ---------------------------------------------------------------------
-- Reference tables
-- ---------------------------------------------------------------------

\echo '==> roles'
INSERT INTO roles (id, name, created_at, updated_at)
SELECT i,
       (ARRAY['admin','moderator','user','guest','banned'])[i],
       '2026-01-01 00:00:00+00'::timestamptz,
       '2026-01-01 00:00:00+00'::timestamptz
FROM generate_series(1, 5) AS i;

\echo '==> tags'
INSERT INTO tags (id, name, created_at, updated_at)
SELECT i,
       'tag-' || lpad(i::text, 5, '0'),
       '2026-01-01 00:00:00+00'::timestamptz,
       '2026-01-01 00:00:00+00'::timestamptz
FROM generate_series(1, :tags) AS i;

-- ---------------------------------------------------------------------
-- Users
-- ---------------------------------------------------------------------
--
-- Every user carries the same valid password hash, so any seeded account
-- can authenticate with one known password. Tier 1 logs in once during
-- warmup to capture a session cookie; login is never inside a measurement,
-- because password hashing would otherwise dominate and the benchmark
-- would be comparing argon2 against bcrypt rather than two frameworks.

\echo '==> users'
INSERT INTO users (id, created_at, updated_at, name, email, password,
                   remember_token, active, deleted_at, email_verified_at)
SELECT i,
       '2026-01-01 00:00:00'::timestamp + (i % 365) * interval '1 day',
       '2026-01-01 00:00:00'::timestamp + (i % 365) * interval '1 day',
       'user-' || lpad(i::text, 8, '0'),
       'user' || i || '@bench.local',
       :'password_hash',
       NULL,
       true,
       NULL,
       '2026-01-01 00:00:00+00'::timestamptz
FROM generate_series(1, :users) AS i;

\echo '==> profiles'
INSERT INTO profiles (id, user_id, bio, created_at, updated_at)
SELECT i,
       i,
       'bio for user ' || i || ' ' || md5(i::text) || md5((i::bigint * 3)::text),
       '2026-01-01 00:00:00+00'::timestamptz,
       '2026-01-01 00:00:00+00'::timestamptz
FROM generate_series(1, :users) AS i;

\echo '==> role_user'
INSERT INTO role_user (id, user_id, role_id, assigned_at, created_at, updated_at)
SELECT i,
       i,
       1 + (('x' || substr(md5(i::text), 1, 8))::bit(32)::bigint & 2147483647) % 5,
       '2026-01-01 00:00:00+00'::timestamptz,
       '2026-01-01 00:00:00+00'::timestamptz,
       '2026-01-01 00:00:00+00'::timestamptz
FROM generate_series(1, :users) AS i;

-- ---------------------------------------------------------------------
-- Posts
-- ---------------------------------------------------------------------
--
-- Body length varies from 100 to ~1900 characters, derived from the row
-- hash. The content is md5 chunks rather than a repeated character, so it
-- does not compress away (see the header). created_at spreads across two
-- years so "recent posts" is a meaningful hot slice rather than the whole
-- table — a feed query should touch a small, cacheable working set while
-- the archive stays cold, which is what a real social app looks like.

\echo '==> posts'
INSERT INTO posts (id, author_id, title, body, is_public, created_at, updated_at)
SELECT i,
       1 + ((i - 1) / :posts_per_user),
       'post ' || i || ' by user ' || (1 + ((i - 1) / :posts_per_user)),
       -- The length cast to int is not cosmetic: the hash arithmetic below
       -- produces bigint, and substr() has no (text, int, bigint) overload.
       substr(
           repeat(md5(i::text) || md5((i::bigint * 7)::text)
                  || md5((i::bigint * 13)::text), 20),
           1,
           (100 + (('x' || substr(md5((i::bigint * 31)::text), 1, 8))::bit(32)::bigint
                   & 2147483647) % 1800)::int
       ),
       -- ~85% public. The filter has to select most but not all rows, or
       -- the planner picks a sequential scan and the index never matters.
       (('x' || substr(md5((i::bigint * 17)::text), 1, 8))::bit(32)::bigint
        & 2147483647) % 100 < 85,
       '2026-01-01 00:00:00'::timestamp
           - (((('x' || substr(md5((i::bigint * 5)::text), 1, 8))::bit(32)::bigint
                & 2147483647) % 730)::int) * interval '1 day',
       '2026-01-01 00:00:00'::timestamp
FROM generate_series(1, :posts) AS i;

-- ---------------------------------------------------------------------
-- Polymorphic children
-- ---------------------------------------------------------------------

\echo '==> comments'
INSERT INTO comments (id, commentable_id, commentable_type, body,
                      created_at, updated_at)
SELECT i,
       1 + ((i - 1) / :comments_per_post),
       'post',
       'comment ' || i || ' ' || md5(i::text) || md5((i::bigint * 11)::text),
       '2026-01-01 00:00:00+00'::timestamptz,
       '2026-01-01 00:00:00+00'::timestamptz
FROM generate_series(1, :comments) AS i;

\echo '==> taggables'
INSERT INTO taggables (id, tag_id, taggable_id, taggable_type)
SELECT i,
       1 + (('x' || substr(md5(i::text), 1, 8))::bit(32)::bigint
            & 2147483647) % :tags,
       1 + ((i - 1) / :tags_per_post),
       'post'
FROM generate_series(1, :taggables) AS i
-- The unique index is (tag_id, taggable_id, taggable_type). Three tags per
-- post drawn from a hash will occasionally collide on the same post; drop
-- the duplicates rather than failing the load.
ON CONFLICT DO NOTHING;

-- Sequences must follow the explicit ids, or the first application INSERT
-- collides with a seeded row. A benchmark that dies on its first write
-- because of this wastes a whole run.
\echo '==> sequences'
SELECT setval(pg_get_serial_sequence('users', 'id'), :users);
SELECT setval(pg_get_serial_sequence('posts', 'id'), :posts);
SELECT setval(pg_get_serial_sequence('comments', 'id'), :comments);
SELECT setval(pg_get_serial_sequence('profiles', 'id'), :users);
SELECT setval(pg_get_serial_sequence('tags', 'id'), :tags);
SELECT setval(pg_get_serial_sequence('roles', 'id'), 5);
SELECT setval(pg_get_serial_sequence('role_user', 'id'), :users);
SELECT setval(pg_get_serial_sequence('taggables', 'id'), :taggables);

RESET synchronous_commit;
