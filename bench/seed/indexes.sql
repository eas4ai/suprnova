-- Benchmark index set. Runs after load.sql, never before.
--
-- Building these incrementally across 400M inserts costs the better part
-- of a day; building them afterwards costs about an hour.
--
-- # Why this file exists at all
--
-- The dogfood app's migrations ship four indexes that are correct for the
-- relations they serve — comments(commentable_id, commentable_type),
-- role_user(user_id, role_id), profiles(user_id), tags(name) — and miss
-- the ones below. At 5,000 rows nothing notices. At 50M rows every one of
-- these is the difference between an index scan and a sequential scan of
-- the whole table, which would make Tier 3 a benchmark of the missing
-- index rather than of the ORM.
--
-- These are applied identically to the Suprnova database and the Laravel
-- database, and recorded in the parity contract. Indexing one side only
-- would rig the comparison; indexing both is building the schema a real
-- application would already have.
--
-- Two of these are arguably defects in the dogfood app rather than
-- benchmark setup — see bench/PLAN.md. Whether the app's own migrations
-- gain them is a separate decision on its own merits.

\set ON_ERROR_STOP on

-- Index builds are memory-bound. The default 64 MB turns a sort that could
-- happen in memory into an external merge across hundreds of temp files.
SET maintenance_work_mem = '8GB';

-- posts.author_id — a HasMany foreign key with no index. Required by
-- Post::for_author (Tier 2.1, Tier 3.2) and by the user.posts eager load.
-- Without it, every author page sequentially scans 50M rows.
\echo '==> posts(author_id)'
CREATE INDEX IF NOT EXISTS idx_posts_author ON posts (author_id);

-- posts(is_public, id) — Tier 1's /api/posts filters on is_public and
-- orders by id. Composite in that order so the filter and the ordering are
-- both served by one scan rather than a filter followed by a sort.
\echo '==> posts(is_public, id)'
CREATE INDEX IF NOT EXISTS idx_posts_public_id ON posts (is_public, id);

-- posts(created_at DESC) — the feed query. A social feed reads recent
-- posts, which is what keeps the hot working set small while the archive
-- stays cold; without this it is a full scan plus a sort of 50M rows.
\echo '==> posts(created_at desc)'
CREATE INDEX IF NOT EXISTS idx_posts_recent ON posts (created_at DESC, id DESC);

-- taggables(taggable_id, taggable_type) — the existing unique index leads
-- with tag_id, which answers "which posts have this tag". The morph-to-many
-- eager load asks the opposite question, "which tags does this post have",
-- and a leading-column mismatch cannot serve it. 150M rows scanned per
-- request without this.
\echo '==> taggables(taggable_id, taggable_type)'
CREATE INDEX IF NOT EXISTS idx_taggables_reverse
    ON taggables (taggable_id, taggable_type);

-- users(email) — the login lookup. Deliberately NOT unique: the
-- application's schema has no uniqueness constraint on email, and adding
-- one here would change write semantics between the benchmark and what the
-- framework actually ships. The lookup performance is the part the
-- benchmark needs; the constraint question belongs to the app.
\echo '==> users(email)'
CREATE INDEX IF NOT EXISTS idx_users_email ON users (email);

-- Statistics drive every plan choice above. A freshly bulk-loaded table has
-- none, so the planner would guess — and an EXPLAIN parity check between
-- the two stacks is meaningless if either side is planning on defaults.
\echo '==> analyze'
ANALYZE users;
ANALYZE posts;
ANALYZE comments;
ANALYZE profiles;
ANALYZE tags;
ANALYZE taggables;
ANALYZE roles;
ANALYZE role_user;

RESET maintenance_work_mem;
