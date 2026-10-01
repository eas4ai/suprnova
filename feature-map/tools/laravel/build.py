"""Assemble laravel/surface.jsonl: every extracted Laravel record, filed under its docs page.

The docs page only decides where a record is tracked; every record comes
from laravel/framework's source through extract.php. Rules are explicit and
ordered: classes by namespace (longest prefix wins), members with their
parent, facades by the implementation their @see names, and everything else
by the rule for its family. A rule naming a page missing from the pinned
docs index stops the run; a record no rule claims goes to a named
"(no page)" group.

Usage: build.py <raw.jsonl> <extract-exclusions.json> <docs-dir> <docs-rev> <laravel-version> <out-dir>
"""
import json
import re
import sys
from collections import Counter
from pathlib import Path

RAW, EXTRACT_EXCL, DOCS, DOCS_REV, VERSION, OUT = sys.argv[1:7]
OUT = Path(OUT)

NAMESPACE_RULES = {
    "Illuminate\\Auth\\Access": "authorization",
    "Illuminate\\Auth\\Passwords": "passwords",
    "Illuminate\\Auth\\Notifications\\ResetPassword": "passwords",
    "Illuminate\\Auth\\Notifications\\VerifyEmail": "verification",
    "Illuminate\\Auth\\Middleware\\EnsureEmailIsVerified": "verification",
    "Illuminate\\Auth\\Middleware\\Authorize": "authorization",
    "Illuminate\\Auth\\MustVerifyEmail": "verification",
    "Illuminate\\Auth": "authentication",
    "Illuminate\\Broadcasting": "broadcasting",
    "Illuminate\\Bus": "queues",
    "Illuminate\\Cache\\RateLimit": "rate-limiting",
    "Illuminate\\Cache": "cache",
    "Illuminate\\Support\\Collection": "collections",
    "Illuminate\\Support\\LazyCollection": "collections",
    "Illuminate\\Support\\Enumerable": "collections",
    "Illuminate\\Support\\HigherOrder": "collections",
    "Illuminate\\Support\\Traits\\EnumeratesValues": "collections",
    "Illuminate\\Support\\Arr": "helpers",
    "Illuminate\\Concurrency": "concurrency",
    "Illuminate\\Config": "configuration",
    "Illuminate\\Console\\Scheduling": "scheduling",
    "Illuminate\\Console": "artisan",
    "Illuminate\\Container": "container",
    "Illuminate\\Contracts": "contracts",
    "Illuminate\\Cookie": "responses",
    "Illuminate\\Database\\Eloquent\\Relations": "eloquent-relationships",
    "Illuminate\\Database\\Eloquent\\Casts": "eloquent-mutators",
    "Illuminate\\Database\\Eloquent\\Concerns\\HasAttributes": "eloquent-mutators",
    "Illuminate\\Database\\Eloquent\\Concerns\\HidesAttributes": "eloquent-serialization",
    "Illuminate\\Database\\Eloquent\\Factories": "eloquent-factories",
    "Illuminate\\Database\\Eloquent\\Collection": "eloquent-collections",
    "Illuminate\\Database\\Eloquent": "eloquent",
    "Illuminate\\Database\\Query": "queries",
    "Illuminate\\Database\\Schema": "migrations",
    "Illuminate\\Database\\Migrations": "migrations",
    "Illuminate\\Database\\Console\\Migrations": "migrations",
    "Illuminate\\Database\\Console\\Seeds": "seeding",
    "Illuminate\\Database\\Seeder": "seeding",
    "Illuminate\\Pagination": "pagination",
    "Illuminate\\Database": "database",
    "Illuminate\\Encryption": "encryption",
    "Illuminate\\Events": "events",
    "Illuminate\\Filesystem": "filesystem",
    "Illuminate\\Foundation\\Testing\\Concerns\\MakesHttpRequests": "http-tests",
    "Illuminate\\Foundation\\Testing\\Concerns\\InteractsWithDatabase": "database-testing",
    "Illuminate\\Foundation\\Testing\\RefreshDatabase": "database-testing",
    "Illuminate\\Foundation\\Testing\\DatabaseTransactions": "database-testing",
    "Illuminate\\Foundation\\Testing\\DatabaseMigrations": "database-testing",
    "Illuminate\\Foundation\\Testing\\DatabaseTruncation": "database-testing",
    "Illuminate\\Foundation\\Testing\\LazilyRefreshDatabase": "database-testing",
    "Illuminate\\Foundation\\Testing\\Concerns\\InteractsWithConsole": "console-tests",
    "Illuminate\\Foundation\\Testing\\Concerns\\InteractsWithContainer": "mocking",
    "Illuminate\\Foundation\\Testing\\Concerns\\InteractsWithTime": "mocking",
    "Illuminate\\Foundation\\Testing\\Wormhole": "mocking",
    "Illuminate\\Foundation\\Testing": "testing",
    "Illuminate\\Foundation\\Http\\FormRequest": "validation",
    "Illuminate\\Foundation\\Http\\Middleware\\HandlePrecognitiveRequests": "precognition",
    "Illuminate\\Foundation\\Precognition": "precognition",
    "Illuminate\\Http\\Concerns\\CanBePrecognitive": "precognition",
    "Illuminate\\Foundation\\Routing\\PrecognitionCallableDispatcher": "precognition",
    "Illuminate\\Foundation\\Routing\\PrecognitionControllerDispatcher": "precognition",
    "Illuminate\\Foundation\\Http\\Middleware\\PreventRequestsDuringMaintenance": "configuration",
    "Illuminate\\Foundation\\Http\\Middleware\\ValidateCsrfToken": "csrf",
    "Illuminate\\Foundation\\Http\\Middleware\\VerifyCsrfToken": "csrf",
    "Illuminate\\Foundation\\Http\\Middleware": "middleware",
    "Illuminate\\Foundation\\Console": "artisan",
    "Illuminate\\Foundation\\Auth\\Access": "authorization",
    "Illuminate\\Foundation\\Auth\\EmailVerificationRequest": "verification",
    "Illuminate\\Foundation\\Auth": "authentication",
    "Illuminate\\Foundation\\Vite": "vite",
    "Illuminate\\Foundation\\Exceptions": "errors",
    "Illuminate\\Foundation\\Support\\Providers": "providers",
    "Illuminate\\Foundation\\Bus": "queues",
    "Illuminate\\Foundation\\Events": "events",
    "Illuminate\\Foundation": "lifecycle",
    "Illuminate\\Hashing": "hashing",
    "Illuminate\\Http\\Client": "http-client",
    "Illuminate\\Http\\Resources": "eloquent-resources",
    "Illuminate\\Http\\Response": "responses",
    "Illuminate\\Http\\JsonResponse": "responses",
    "Illuminate\\Http\\RedirectResponse": "responses",
    "Illuminate\\Http\\StreamedEvent": "responses",
    "Illuminate\\Http\\Middleware": "middleware",
    "Illuminate\\Http": "requests",
    "Illuminate\\Image": "images",
    "Illuminate\\Log\\Context": "context",
    "Illuminate\\Log": "logging",
    "Illuminate\\Mail": "mail",
    "Illuminate\\Notifications": "notifications",
    "Illuminate\\Pipeline": "helpers",
    "Illuminate\\Process": "processes",
    "Illuminate\\Queue": "queues",
    "Illuminate\\Redis": "redis",
    "Illuminate\\Routing\\UrlGenerator": "urls",
    "Illuminate\\Routing\\Middleware\\ValidateSignature": "urls",
    "Illuminate\\Routing\\Middleware\\ThrottleRequests": "rate-limiting",
    "Illuminate\\Routing\\Controller": "controllers",
    "Illuminate\\Routing\\Redirector": "responses",
    "Illuminate\\Routing\\ResponseFactory": "responses",
    "Illuminate\\Routing": "routing",
    "Illuminate\\Session": "session",
    "Illuminate\\Support\\Facades": "facades",
    "Illuminate\\Support\\Str": "strings",
    "Illuminate\\Support\\Stringable": "strings",
    "Illuminate\\Support\\ServiceProvider": "providers",
    "Illuminate\\Support\\AggregateServiceProvider": "providers",
    "Illuminate\\Support\\DefaultProviders": "providers",
    "Illuminate\\Support\\Testing\\Fakes": "mocking",
    "Illuminate\\Support\\Env": "configuration",
    "Illuminate\\Support": "helpers",
    "Illuminate\\Testing\\PendingCommand": "console-tests",
    "Illuminate\\Testing\\ParallelTesting": "testing",
    "Illuminate\\Testing": "http-tests",
    "Illuminate\\Translation": "localization",
    "Illuminate\\Validation": "validation",
    "Illuminate\\View\\Compilers": "blade",
    "Illuminate\\View\\Component": "blade",
    "Illuminate\\View\\AnonymousComponent": "blade",
    "Illuminate\\View\\DynamicComponent": "blade",
    "Illuminate\\View": "views",
    "Illuminate\\JsonSchema": "(no page) JSON schema builder",
}
ARTISAN_RULES = [  # (command prefix, page); first match wins
    ("migrate", "migrations"), ("schema:", "migrations"), ("make:migration", "migrations"),
    ("db:seed", "seeding"), ("make:seeder", "seeding"), ("db:", "database"),
    ("queue:", "queues"), ("make:job", "queues"), ("make:job-middleware", "queues"),
    ("schedule:", "scheduling"), ("route:", "routing"), ("install:api", "routing"),
    ("install:broadcasting", "broadcasting"), ("make:channel", "broadcasting"),
    ("config:", "configuration"), ("env:", "configuration"), ("optimize", "deployment"),
    ("down", "configuration"), ("up", "configuration"), ("cache:", "cache"), ("make:cache-table", "cache"),
    ("view:", "views"), ("make:view", "views"), ("make:component", "blade"),
    ("event:", "events"), ("make:event", "events"), ("make:listener", "events"),
    ("storage:", "filesystem"), ("key:generate", "encryption"), ("model:", "eloquent"),
    ("make:model", "eloquent"), ("make:observer", "eloquent"), ("make:scope", "eloquent"),
    ("make:cast", "eloquent-mutators"), ("make:factory", "eloquent-factories"),
    ("make:resource", "eloquent-resources"), ("make:controller", "controllers"),
    ("make:middleware", "middleware"), ("make:request", "validation"), ("make:rule", "validation"),
    ("make:mail", "mail"), ("make:notification", "notifications"), ("make:notifications-table", "notifications"),
    ("make:policy", "authorization"), ("make:provider", "providers"), ("make:session-table", "session"),
    ("session:", "session"), ("make:test", "testing"), ("test", "testing"),
    ("lang:", "localization"), ("vendor:publish", "packages"), ("auth:clear-resets", "passwords"),
    ("make:command", "artisan"), ("make:", "artisan"),
]
CONFIG_PAGES = {
    "app": "configuration", "auth": "authentication", "broadcasting": "broadcasting", "cache": "cache",
    "concurrency": "concurrency", "cors": "routing", "database": "database", "filesystems": "filesystem",
    "hashing": "hashing", "images": "images", "logging": "logging", "mail": "mail", "queue": "queues",
    "services": "configuration", "session": "session", "view": "views",
}

docs_pages = set(re.findall(r'\(/docs/\{\{version\}\}/([a-z0-9-]+)\)', (Path(DOCS) / "documentation.md").read_text()))
targets = list(NAMESPACE_RULES.values()) + [p for _, p in ARTISAN_RULES] + list(CONFIG_PAGES.values()) + [
    "facades", "helpers", "blade", "validation", "configuration", "artisan"]
missing = sorted({t for t in targets if not t.startswith("(no page)") and t not in docs_pages})
if missing:
    raise SystemExit(f"rules name docs pages missing from the pinned index: {missing}")


def by_namespace(cls):
    best = max((p for p in NAMESPACE_RULES if cls == p or cls.startswith(p + "\\") or cls.startswith(p)),
               key=len, default=None)
    return NAMESPACE_RULES[best] if best else None


records = [json.loads(l) for l in Path(RAW).read_text().splitlines() if l.strip()]
by_id = {r["id"]: r for r in records}


def page_of(r):
    kind, rid = r["kind"], r["id"]
    if kind == "helper":
        return "helpers"
    if kind == "blade directive":
        return "blade"
    if kind == "validation rule":
        return "validation"
    if kind == "env var":
        return "configuration"
    if kind == "config key":
        return CONFIG_PAGES.get(rid.split(" ", 1)[1].split(".")[0])
    if kind == "artisan command":
        name = rid.split(" ", 1)[1]
        return next((p for pre, p in ARTISAN_RULES if name == pre or name.startswith(pre)),
                    by_namespace(r["details"].get("class", "")))
    if rid.startswith("Illuminate\\Support\\Facades\\") and r["parent"] is None:
        for see in r["details"].get("see", []):
            page = by_namespace(see.lstrip("\\"))
            if page and page != "facades":
                return page
    return by_namespace(rid)


# A rule that claims nothing is misleading; every namespace rule must match a record.
unused = [p for p in NAMESPACE_RULES
          if not any(r["id"] == p or r["id"].startswith(p) for r in records if r["parent"] is None)]
if unused:
    raise SystemExit(f"namespace rules that match no record: {unused}")

pages = {}
for r in records:
    if r["parent"] is None:
        pages[r["id"]] = page_of(r) or "(no page) not yet classified"
out = []
for r in records:
    r = dict(r)
    parent = r["parent"]
    r["page"] = pages.get(parent) if parent else pages[r["id"]]
    if r["page"] is None:  # a member whose parent was excluded
        r["page"] = "(no page) not yet classified"
    out.append(r)
out.sort(key=lambda r: r["id"])

OUT.mkdir(parents=True, exist_ok=True)
(OUT / "surface.jsonl").write_text("".join(json.dumps(r, sort_keys=True) + "\n" for r in out))
empty_pages = sorted(docs_pages - {r["page"] for r in out})
excl = json.loads(Path(EXTRACT_EXCL).read_text())
excl["docs_pages_with_no_framework_surface"] = empty_pages
(OUT / "exclusions.json").write_text(json.dumps(excl, indent=1, sort_keys=True) + "\n")
(OUT / "meta.json").write_text(json.dumps({
    "laravel_version": VERSION, "docs_rev": DOCS_REV, "records": len(out),
    "source": "laravel/framework", "docs": "laravel/docs 13.x",
}, indent=1, sort_keys=True) + "\n")
unclassified = [r["id"] for r in out if r["page"] == "(no page) not yet classified" and r["parent"] is None]
print(json.dumps({"records": len(out), "pages": len({r["page"] for r in out if not r["page"].startswith("(")}),
                  "by_page_top": Counter(r["page"] for r in out).most_common(8),
                  "unclassified": len(unclassified), "unclassified_sample": unclassified[:15]}))
