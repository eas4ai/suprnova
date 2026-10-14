# Eloquent collections

`Collection<T>` is Suprnova's Laravel-shape collection type - the
return value of `Builder::get`, `Model::all`, every `pluck`, every
relation-load terminal that yields more than one row. It is a thin
wrapper around `Vec<T>` that derefs to `&[T]`, so every existing
slice method (`.len()`, `.iter()`, indexing, `.contains(&v)`) works
without change. Layered on top is the Laravel surface: `map`,
`filter`, `pluck`, `group_by`, `sort_by`, `where_eq`, `sum`, `avg`,
the lot.

This chapter is the standalone reference for the collection surface.
The parent [Eloquent API](eloquent.md) summarises it; this chapter
goes through every method, the borrow-vs-consume contract, the
shared serialization policy, and when to drop down
to `Vec<T>` instead.

## Table of contents

- [Where collections come from](#where-collections-come-from)
- [The two impl blocks](#the-two-impl-blocks)
- [Generic surface - works on any `Collection<T>`](#generic-surface--works-on-any-collectiont)
- [Model-aware surface - `Collection<M>` where `M: Model`](#model-aware-surface--collectionm-where-m-model)
- [Eager loading on a collection](#eager-loading-on-a-collection)
- [Serialization - `to_array` vs serde](#serialization--to_array-vs-serde)
- [Borrow vs consume](#borrow-vs-consume)
- [Collection vs `Vec`](#collection-vs-vec)
- [`LazyCollection<M>` - streaming results](#lazycollectionm--streaming-results)
- [Why Suprnova diverges](#why-suprnova-diverges)
- [Next](#next)

## Where collections come from

Any terminal that returns more than one row hands you a
`Collection<M>`:

```rust
use suprnova::{Collection, Model};

let users: Collection<User> = User::all().await?;
let admins: Collection<User> = User::query()
    .db_where("role", "=", "admin")
    .get()
    .await?;
let recent: Collection<User> = User::query()
    .order_by_desc("created_at")
    .limit(50)
    .get()
    .await?;
```

You can also wrap any `Vec<T>` you already have:

```rust
let from_vec: Collection<User> = users_vec.into();
let from_vec2: Collection<User> = Collection::from_vec(users_vec);
let empty: Collection<User> = Collection::new();
```

`Collection<T>` implements `Default`, `Clone`, `Serialize`,
`Deserialize`, `PartialEq`, and `IntoIterator` (both by-value and by
`&`). It is `Send` when `T: Send`.

## The two impl blocks

The methods on `Collection` split into two families based on the type
parameter.

```rust
impl<T> Collection<T> { /* generic methods - work for any T */ }

impl<M> Collection<M> where M: Model { /* string-keyed model methods */ }
```

The generic block gives you `map`, `filter`, `reject`, `chunk`,
`first`, `last`, `unique`, and a closure-based version of every
column accessor (`pluck_by`, `group_by_with`, `sort_with`,
`key_by_with`). These work on `Collection<i32>`,
`Collection<String>`, `Collection<MyDto>`, anything.

The model-aware block adds string-keyed sugar (`pluck("name")`,
`group_by("role")`, `sort_by("created_at")`, `sum::<f64>("balance")`)
that routes per-row through the macro-emitted `Model::field_value`
accessor. These only exist when `T` implements `Model`.

Pick the closure form when you can - the type checker validates the
field access. Pick the string-keyed form when you're matching
Laravel's syntax, or when the column name is a runtime value.

## Generic surface - works on any `Collection<T>`

### Reading

```rust
use suprnova::Collection;

let nums: Collection<i32> = Collection::from_vec(vec![3, 1, 4, 1, 5, 9, 2, 6]);

nums.len();                         // 8
nums.is_empty();                    // false
nums.is_not_empty();                // true
nums.first();                       // Some(&3)
nums.last();                        // Some(&6)
nums.first_where(|n| **n > 3);      // Some(&4)
nums.last_where(|n| **n > 3);       // Some(&6)
nums.contains(&4);                  // true - from Deref<Target = [T]>
nums.contains_where(|n| *n > 5);    // true
```

`first_where` / `last_where` take `&&T` because the predicate runs
through `Iterator::find` on `Iter<'_, T>`. Dereference twice (`**n`).

### Transforming - consume `self`, return new collection

```rust
let doubled: Collection<i32>      = nums.clone().map(|n| n * 2);
let evens:   Collection<i32>      = nums.clone().filter(|n| n % 2 == 0);
let odds:    Collection<i32>      = nums.clone().reject(|n| n % 2 == 0);
let unique:  Collection<i32>      = nums.clone().unique();
let chunks:  Vec<Collection<i32>> = nums.clone().chunk(3);
let taken:   Collection<i32>      = nums.clone().take(4);
let skipped: Collection<i32>      = nums.clone().skip(2);
let middle:  Collection<i32>      = nums.clone().slice(2, 4);
let flipped: Collection<i32>      = nums.clone().reverse();
let shuffled: Collection<i32>     = nums.clone().shuffle();
```

`map` changes the element type:

```rust
let labels: Collection<String> = nums.clone().map(|n| format!("n={n}"));
```

`each` runs a side effect and keeps the collection for further
chaining (Suprnova diverges from Laravel here on purpose - see below):

```rust
let kept = nums.clone()
    .each(|n| tracing::debug!(value = n, "processing"))
    .filter(|n| *n > 2)
    .take(3);
```

### Closure-keyed grouping and sorting

```rust
use std::collections::HashMap;
use suprnova::indexmap::IndexMap;

// Bucket items by closure-derived key.
let by_parity: HashMap<bool, Collection<i32>> =
    nums.clone().group_by_with(|n| n % 2 == 0);

// Index items by closure-derived key (later duplicates overwrite).
let by_value: HashMap<i32, i32> =
    nums.clone().key_by_with(|n| *n);

// Sort by closure-derived comparator.
let sorted_desc: Collection<i32> =
    nums.clone().sort_with(|a, b| b.cmp(a));

// Sort `Ord` items from greatest to least; equal items keep their order.
let greatest_first: Collection<i32> = nums.clone().sort_desc();

// Project every item into a key and a value, in the collection's order.
let labels: IndexMap<i32, String> =
    nums.clone().map_with_keys(|n| (n, format!("n={n}")));

// Deduplicate by closure-derived key.
let unique_mod3: Collection<i32> =
    nums.clone().unique_by(|n| n % 3);

// Project every item by closure into a new collection.
let strs: Collection<String> =
    nums.pluck_by(|n| n.to_string());
```

`sort_desc` is Laravel's `sortDesc`: a stable sort with the comparison
reversed, so two items that compare equal stay in the order they had,
as PHP's sort keeps them. `map_with_keys` is Laravel's `mapWithKeys`: it
collects into an `IndexMap` (re-exported as `suprnova::indexmap`) in the
collection's order, and a repeated key keeps the position of its first
pair and the value of its last. `map_to_map` collects the same pairs
into a `HashMap`, which keeps no order:

```rust
use suprnova::indexmap::IndexMap;

let pairs = Collection::from_vec(vec![("a", 1), ("b", 2), ("a", 3)]);
let mapped: IndexMap<&str, i32> = pairs.map_with_keys(|(key, value)| (key, value));
assert_eq!(mapped.keys().copied().collect::<Vec<_>>(), ["a", "b"]);
assert_eq!(mapped["a"], 3);
```

The `*_with` / `*_by` suffix is the universal "this method takes a
closure" naming convention across the generic block. The
model-aware block drops the suffix and takes a column name string
instead.

### Folding and aggregating

```rust
let sum: i32 = nums.clone().reduce(0, |acc, n| acc + n);  // 31
```

For typed numeric aggregates on model collections, see `sum` / `avg`
/ `min` / `max` in the model-aware section - they work on any field
that deserialises to a numeric type.

### Set operations

```rust
let a = Collection::from_vec(vec![1, 2, 3, 4]);
let b = Collection::from_vec(vec![3, 4, 5, 6]);

let joined = a.clone().concat(b.clone());    // [1,2,3,4,3,4,5,6]
let same   = a.clone().merge(b.clone());     // alias of concat
let only_a = a.clone().diff(b.clone());      // [1,2]
let common = a.clone().intersect(b.clone()); // [3,4]
```

`concat` / `merge` are aliases - Laravel ships both names. `diff` /
`intersect` are O(n*m); if you have large collections, project to a
`HashSet` first.

### Random sampling

```rust
let one: Option<&i32>     = nums.random();        // borrow one
let many: Collection<i32> = nums.clone().random_n(3); // pick 3
```

Both use the thread-local RNG (`rand::rng()`). Pass through a seeded
RNG manually if you need determinism in tests.

## Model-aware surface - `Collection<M>` where `M: Model`

These methods only exist when the contained type is a Suprnova
model. They route per-row reads through the macro-emitted
`Model::field_value(name)` accessor, which returns
`Option<serde_json::Value>`. Rows whose field doesn't exist or
doesn't deserialise into the target type are silently skipped -
matching Laravel's missing-key behaviour.

### Lookup and identity

You find loaded models by their primary key without issuing a query:

```rust
let user: Option<&User> = users.find(7);
let same: Option<&User> = users.find_model(&another_user);
let selected: Collection<User> = users.find_many([7, 9]);
let chosen: &User = users.find_or(7, &fallback_user);
```

You get the first matching model from `find` and `find_model`. You get
matching rows in collection order from `find_many`. Repeated requested
keys do not add rows. A missing key returns `None`, an empty selection,
or your fallback. You can use a text key on a model with a custom key.

You compare model identity with `unique_models` and `diff_models`:

```rust
let unique = users.clone().unique_models();
let remaining = users.clone().diff_models(&other_users);
```

You keep the last model for each key with `unique_models`, in the order
of each key's first appearance. Models with null keys are omitted.
You remove rows whose keys occur in `other_users` with `diff_models`,
even when their other attributes differ. You retain the generic
`unique` and `diff` methods for whole-value comparisons.

### Projection

```rust
use suprnova::{Collection, Model};

let users: Collection<User> = User::query().get().await?;

let emails: Collection<String> = users.pluck::<String>("email");
let ids:    Collection<i64>    = users.pluck::<i64>("id");
```

`pluck` borrows (`&self`), so the original collection is still
available afterwards. The typed parameter (`::<String>`) is the
target type the JSON value gets deserialised into.

`pluck_keyed` produces a `HashMap<K, V>` from two columns:

```rust
use std::collections::HashMap;

let email_by_id: HashMap<i64, String> =
    users.pluck_keyed::<i64, String>("id", "email");
```

Later rows overwrite earlier ones for the same key.

`model_keys` is the primary-key shortcut, and the only projection that
returns a plain `Vec` rather than a `Collection`:

```rust
let users: Collection<User> = User::query().get().await?;
let ids: Vec<i64> = users.model_keys();
```

It reads the already-hydrated key field, so it costs no query. When you
only want the keys and haven't loaded the rows yet, use the builder
terminal instead - `User::query().model_keys().await?` projects the key
column without hydrating anything. `Vec` rather than `Collection`
matches Laravel's `modelKeys()`, and keeps the two halves of the pair
agreeing on one shape.

### Grouping and indexing

```rust
use std::collections::HashMap;

let by_role: HashMap<String, Collection<User>> = users.group_by("role");
let by_id:   HashMap<String, User>             = users.key_by("id");
```

Both methods stringify the column value into a `String` key. A
numeric `id` column comes through as `"1"` / `"2"` - matching
Laravel's `groupBy('team_id')` contract where the output is always
string-keyed regardless of the underlying type.

If you want typed keys, use the closure form on the generic block:

```rust
let by_id: HashMap<i64, User> = users.key_by_with(|u| u.id);
```

### Filtering

The model-aware `where_*` methods take `serde_json::Value` because
they compare against the JSON-encoded form of the column:

```rust
use serde_json::json;

let active: Collection<User>  = users.clone().where_eq("active", json!(true));
let admins: Collection<User>  = users.clone()
    .where_in("role", vec![json!("admin"), json!("owner")]);
let non_guests: Collection<User> = users.clone()
    .where_not_in("role", vec![json!("guest")]);
```

`where_eq` and `where_in` drop rows whose `field_value` returns
`None`. `where_not_in` *keeps* rows where the field is missing - the
negation of "in the set" is "not in the set OR absent".

### Sorting

```rust
let by_name_asc:  Collection<User> = users.clone().sort_by("name");
let by_name_desc: Collection<User> = users.clone().sort_by_desc("name");
```

Comparison is best-effort across JSON value shapes: numeric vs
numeric and string vs string sort cleanly within their kind; mixed
heterogeneous columns fall back to `Ordering::Equal`. `None` sorts
before any present value (mirrors Postgres `NULLS FIRST` for ASC).

Both methods clone the underlying `Vec<M>` before sorting because the
comparator borrows `m.field_value(field)` while `sort_by` needs
`&mut [M]`. If you have a tight loop, sort with `sort_with` on the
generic block instead - it operates in place.

### Aggregates

```rust
let total: f64           = users.sum::<f64>("balance");
let avg:   Option<f64>   = users.avg::<f64>("balance");
let lo:    Option<i64>   = users.min::<i64>("login_count");
let hi:    Option<i64>   = users.max::<i64>("login_count");
```

`sum` returns `T::default()` when no row contributes a value (zero
for numeric types). The other three return `None` so the caller
doesn't divide by zero or compare against a phantom default.

The typed parameter (`::<f64>`) is the JSON deserialisation target.
Pick the widest numeric type your column reasonably uses -
`i64` for integer columns, `f64` for decimal/float, `chrono::DateTime<Utc>`
for timestamps, etc.

## Eager loading on a collection

When you already have a `Collection<M>` and want to load relations
onto every row, use `load` / `load_missing`:

```rust
let mut users: Collection<User> = User::query().get().await?;
users.load(["posts.comments"]).await?;

for u in &users {
    for p in u.posts_loaded() {
        println!("{}: {} comments", p.title, p.comments_loaded().len());
    }
}
```

Both methods take `&mut self` (they mutate the per-row eager-cache)
and `async`. Both accept the same dotted-path syntax
`Builder::with([...])` accepts - `"posts"`, `"posts.comments"`,
`"posts.comments.author"`.

`load_missing` partitions per row. Rows that already have the
relation cached are left alone; rows that don't get the bulk-load:

```rust
let mut users: Collection<User> = User::query().with(["posts"]).get().await?;
// Some rows already have posts cached. load_missing only touches the
// rest - and recurses into already-cached posts for `comments`.
users.load_missing(["posts.comments"]).await?;
```

The recursion runs at every segment of a longer dotted path. With
`"a.b.c"`, each row is partitioned at every level: `a` is loaded only
where missing, then for the rows that already had `a`, `b` is loaded
only where missing on those `a`s, etc.

Both methods honour `#[model(connection = "...")]` routing - they
resolve the same connection the row was originally loaded from.

You constrain a relation on the models you already loaded with `load_with`:

```rust
use suprnova::Builder;

users.load_with("posts", |query: Builder<Post>| {
    query.filter("published", true)
}).await?;
```

You replace any cached value for that relation with the constrained result.
You use the relation's target model as the closure's builder type. An
unknown relation or a mismatched target returns an error. You use a flat
relation name, as you do with the builder's constrained eager loading.
With an empty collection, you skip connection resolution and the constraint.

## Serialization - `to_array` vs serde

You get the same model visibility and appends policy through `to_array`,
`to_json`, and serde:

```rust
let value: serde_json::Value = users.to_array();
let body: String = users.to_json();
let serde_body: String = serde_json::to_string(&users)?;
```

You keep hidden fields out of every row, including rows nested in Inertia
props or another serde container. You include an appended accessor only
when the model's hidden and visible lists permit its name. Instance changes
made with `make_hidden`, `make_visible`, or `append` apply to later outputs.
For non-model elements, you use their ordinary serde serialization.

## Borrow vs consume

The methods split cleanly into two contracts:

| Takes | Methods |
|---|---|
| `&self` (borrow) | `len`, `is_empty`, `is_not_empty`, `first`, `last`, `first_where`, `last_where`, `contains_where`, `random`, `find`, `find_model`, `find_many`, `find_or`, `as_slice`, `pluck_by`, `pluck`, `pluck_keyed`, `group_by`, `key_by`, `sum`, `avg`, `min`, `max`, `to_array`, `to_json` |
| `self` (consume) | `map`, `filter`, `reject`, `each`, `reduce`, `chunk`, `take`, `skip`, `slice`, `reverse`, `shuffle`, `random_n`, `unique`, `unique_models`, `diff_models`, `unique_by`, `sort_with`, `sort_desc`, `sort_by`, `sort_by_desc`, `where_eq`, `where_in`, `where_not_in`, `concat`, `merge`, `diff`, `intersect`, `group_by_with`, `key_by_with`, `map_to_map`, `map_with_keys` |
| `&mut self` | `load`, `load_missing`, `load_with` |

If you want to keep the collection after a consuming call, `.clone()`
before the call. `Collection<T>: Clone` when `T: Clone`.

A practical pattern: read first, then transform last:

```rust
let users: Collection<User> = User::all().await?;

// Borrowing reads first - the collection is still alive after each.
let total       = users.sum::<f64>("balance");
let avg         = users.avg::<f64>("balance");
let count_admin = users.iter().filter(|u| u.role == "admin").count();
let emails      = users.pluck::<String>("email");

// Now consume.
let admins: Collection<User> = users.where_eq("role", json!("admin"));
```

## Collection vs `Vec`

The wrapper is intentionally thin. The conversion routes go both
ways and stay cheap:

```rust
let v: Vec<User>          = User::query().get().await?.into_vec();
let c: Collection<User>   = Collection::from(v);
let c2: Collection<User>  = Collection::from_vec(c.clone().into_vec());
```

`Deref<Target = [T]>` gives you every slice method automatically.
That includes:

```rust
let users: Collection<User> = User::all().await?;

users.len();             // slice method
users.iter();            // slice method
users[0].name.clone();   // slice indexing
users.contains(&u);      // slice method
users.binary_search(&u); // slice method
&users[1..4];            // slice subscripting
```

`IntoIterator` is implemented twice - for `Collection<T>` (by value)
and `&Collection<T>` (by reference), so both of these work:

```rust
for user in &users {           // iter by &User
    /* ... */
}

for user in users.clone() {    // iter by User (consumes)
    /* ... */
}
```

`DerefMut` only yields `&mut [T]` - a slice, not a `Vec`. That means
in-place mutation of element fields works:

```rust
let mut users: Collection<User> = User::all().await?;
for u in users.iter_mut() {
    u.last_seen_at = Some(Utc::now());
}
```

But owned `Vec` mutation (`push`, `pop`, `clear`, `truncate`) is not
available on the collection directly - call `into_vec()` first:

```rust
let mut v = users.into_vec();
v.push(new_user);
let users: Collection<User> = Collection::from(v);
```

That's deliberate. The Laravel surface treats a collection as an
immutable snapshot you transform with chained methods; owned mutation
of the inner sequence is the `Vec` contract, not the `Collection`
contract.

### When to drop to `Vec`

Reach for `into_vec()` when:

- You need `Vec`-specific methods (`push`, `pop`, `swap_remove`,
  `drain`, `with_capacity`).
- You're handing the data off to an API that takes `Vec<T>` by value
  and you don't want the wrapper in the signature.
- You're storing the rows long-term in your own struct and the
  Laravel surface buys you nothing.

For everything else - handler returns, transformations, Inertia
props -
keep the `Collection<T>`.

## `LazyCollection<M>` - streaming results

`Collection<M>` materialises every row in memory. For datasets too
large to fit, the builder offers three streaming terminals that
return `LazyCollection<M>` instead:

```rust
use suprnova::Model;

let mut stream = User::query().lazy();
while let Some(row) = stream.next().await {
    let user = row?;
    println!("{}", user.email);
}
```

| Method | Strategy |
|---|---|
| `Builder::lazy()` | PK-cursor pagination with the default batch size (1000) |
| `Builder::lazy_by_id(n)` | PK-cursor pagination with batch size `n` |
| `Builder::cursor()` | Laravel alias for `lazy()` |

`LazyCollection<M>` is a `Pin<Box<dyn Stream<Item = Result<M, FrameworkError>> + Send>>`
underneath, but exposes `.next().await` directly so you don't need
to import `futures::StreamExt`. Each `.next()` triggers the next row
delivery; the underlying batched fetch only runs when the in-batch
buffer drains, so a slow consumer doesn't accumulate rows.

The wrapper is `Send` (so it crosses `tokio::spawn`) but not
`Sync` - it's a single-consumer stream by construction.

See [Eloquent - chunking and lazy iteration](eloquent.md#chunking-and-lazy-iteration)
for the full guidance on which streaming pattern to pick.

## Why Suprnova diverges

Laravel's `Illuminate\Support\Collection` is mutable: `$c->filter(...)`
modifies the inner array of the same object and returns `$this` for
chaining. PHP doesn't have ownership, so that contract is invisible.

Rust does have ownership, and pretending it doesn't would make the
collection surface dishonest. Suprnova picks the value-semantic
shape instead: every transformation consumes `self` and returns a
new `Collection`. You see the cost in your own code - if you want
to keep the original, you `.clone()`. If you don't, you don't.

That choice cascades through the rest of the surface:

- **`each` returns `Self`** instead of `&self` so a side-effect
  call (logging, metrics) doesn't break a chain. PHP's `each` runs
  for-effect and returns the collection; you couldn't do
  `$c->each(...)->filter(...)` cleanly without re-fetching. In
  Rust we move `self` through, keeping the chain fluent.

- **Closure-keyed alternatives to every string-keyed method.**
  `pluck_by`, `group_by_with`, `key_by_with`, `sort_with`,
  `unique_by`, `map_to_map`, `contains_where`. The closures let
  you read fields the type checker validates instead of strings
  the compiler can't see. The string-keyed forms exist for
  Laravel-syntax parity and for runtime-decided column names.

- **`sum` / `avg` / `min` / `max` take typed `::<T>` parameters.**
  Laravel's PHP version casts on the fly; in Rust, the
  deserialisation target is part of the call. Rows whose value
  doesn't round-trip into `T` are silently skipped (matching
  Laravel's missing-key behaviour), but you pick the type
  intentionally.

- **`Deref<Target = [T]>`, not `Deref<Target = Vec<T>>`.** A
  `Collection` is conceptually a "snapshot of rows", not a
  mutable buffer. Slice methods come through `Deref`; if you
  want `push`/`pop`, `into_vec()` gives you the raw `Vec` and
  removes any pretence.

- **Separate identity methods.** You use `unique_models` and
  `diff_models` for primary-key comparisons. You keep the generic
  `unique` and `diff` methods for whole-value comparisons because Rust
  does not overload inherent methods by trait bounds.

The trade-off is exactly the one Suprnova makes everywhere: Laravel's
surface shape, Rust's value semantics.

## Next

- [Eloquent API](eloquent.md) - the parent chapter, with the
  query builder, relations, scopes, and the full model lifecycle.
- [JSON:API resources](eloquent-resources.md) - resource structs
  serialise collections through `IntoJsonResource` with sparse
  fieldsets and `?include=` chains; the right shape for any
  collection that leaves your API.
- [Frontend - Inertia responses](frontend-inertia-responses.md) -
  how you hand collections to typed Inertia props.
- [Validation](validation.md) - request payloads frequently produce
  vectors that you wrap into `Collection` for downstream
  processing.
- [Testing](testing.md) - patterns for asserting on collection
  contents (length, contained elements, ordering) inside handler
  and model tests.
