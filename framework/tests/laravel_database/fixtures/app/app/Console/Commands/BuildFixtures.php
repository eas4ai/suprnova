<?php

namespace App\Console\Commands;

use App\Jobs\FailingJob;
use App\Jobs\ProcessPodcast;
use App\Models\Post;
use App\Models\User;
use App\Notifications\InvoicePaid;
use Composer\InstalledVersions;
use Illuminate\Console\Command;
use Illuminate\Database\Eloquent\Relations\Relation;
use Illuminate\Database\Events\QueryExecuted;
use Illuminate\Session\DatabaseSessionHandler;
use Illuminate\Session\Store;
use Illuminate\Support\Facades\Artisan;
use Illuminate\Support\Facades\Auth;
use Illuminate\Support\Facades\Bus;
use Illuminate\Support\Facades\Cache;
use Illuminate\Support\Facades\DB;
use Illuminate\Support\Facades\Hash;
use Illuminate\Support\Facades\Schema;
use Laravel\Pennant\Feature;
use PDO;
use RuntimeException;
use Spatie\Permission\Models\Permission;
use Spatie\Permission\Models\Role;

// Builds one Suprnova laravel_database fixture: the schema Laravel's own
// migrations create on the current connection, and rows Laravel itself
// writes, dumped as SQL statements in a JSON file.
class BuildFixtures extends Command
{
    protected $signature = 'fixtures:build {engine : sqlite, mysql or pgsql} {out : the JSON file to write}';

    protected $description = 'Migrate, seed through Laravel, and dump the Suprnova laravel_database fixture';

    // The plaintext passwords the users are created with. The tests sign in
    // with them, so they are written into the fixture. Abigail's is 72 bytes;
    // Dayle's is 100 bytes, and its 72nd byte is the first byte of a two-byte
    // character, so a hasher that cuts at 72 bytes cuts inside it.
    private function passwords(): array
    {
        return [
            'taylor@example.com' => 'password',
            'abigail@example.com' => str_repeat('0123456789', 7).'ab',
            'dayle@example.com' => 'x'.str_repeat("\u{e9}", 49).'x',
            'jeffrey@example.com' => 'secret',
        ];
    }

    public function handle(): int
    {
        $engine = $this->argument('engine');
        $driver = DB::connection()->getDriverName();
        if ($engine !== $driver) {
            throw new RuntimeException("asked for {$engine} but the default connection is {$driver}");
        }
        $this->checkPasswords();

        $schema = $this->migrate();
        $sessions = $this->seed();
        $rows = $this->dumpRows($this->createdTables($schema));

        $fixture = [
            'engine' => $engine,
            'generated_with' => [
                'php' => PHP_VERSION,
                'laravel/framework' => InstalledVersions::getPrettyVersion('laravel/framework'),
                'laravel/pennant' => InstalledVersions::getPrettyVersion('laravel/pennant'),
                'spatie/laravel-permission' => InstalledVersions::getPrettyVersion('spatie/laravel-permission'),
                'server' => $this->serverVersion(),
            ],
            'schema' => $schema,
            'rows' => $rows,
            'users' => $this->users(),
            'sessions' => $sessions,
            'morph' => ['class' => Post::class, 'alias' => 'post'],
        ];

        $json = json_encode($fixture, JSON_PRETTY_PRINT | JSON_UNESCAPED_SLASHES | JSON_UNESCAPED_UNICODE | JSON_THROW_ON_ERROR);
        file_put_contents($this->argument('out'), $json."\n");
        $this->info('wrote '.$this->argument('out'));

        return self::SUCCESS;
    }

    // The fixed passwords must have the byte lengths the tests rely on.
    private function checkPasswords(): void
    {
        $expected = ['taylor@example.com' => 8, 'abigail@example.com' => 72, 'dayle@example.com' => 100, 'jeffrey@example.com' => 6];
        foreach ($expected as $email => $bytes) {
            if (strlen($this->passwords()[$email]) !== $bytes) {
                throw new RuntimeException("the password for {$email} is ".strlen($this->passwords()[$email])." bytes, not {$bytes}");
            }
        }
    }

    // Runs Laravel's migrations and returns the schema statements they ran,
    // in order: every executed statement whose first word is CREATE, ALTER,
    // DROP or COMMENT.
    private function migrate(): array
    {
        $statements = [];
        DB::listen(function (QueryExecuted $query) use (&$statements) {
            $statements[] = $query;
        });
        $status = Artisan::call('migrate', ['--force' => true]);
        if ($status !== 0) {
            throw new RuntimeException('migrate failed: '.Artisan::output());
        }

        $schema = [];
        foreach ($statements as $query) {
            $sql = trim($query->sql);
            if (! preg_match('/^(create|alter|drop|comment)\b/i', $sql)) {
                continue;
            }
            if ($query->bindings !== []) {
                throw new RuntimeException("a schema statement has bindings: {$sql}");
            }
            $schema[] = $sql;
        }
        if ($schema === []) {
            throw new RuntimeException('migrate ran no schema statement');
        }

        return $schema;
    }

    // Writes every row through Laravel. Returns the ids of the two sessions.
    private function seed(): array
    {
        // Users. Hash::make is Laravel's default bcrypt hasher ($2y$, cost 12).
        $taylor = User::create([
            'name' => 'Taylor',
            'email' => 'taylor@example.com',
            'password' => Hash::make($this->passwords()['taylor@example.com']),
        ]);
        $taylor->forceFill([
            'email_verified_at' => now(),
            'remember_token' => str_pad('laravel-remember-token-', 60, '0123456789'),
        ])->save();
        $abigail = User::create([
            'name' => 'Abigail',
            'email' => 'abigail@example.com',
            'password' => Hash::make($this->passwords()['abigail@example.com']),
        ]);
        $dayle = User::create([
            'name' => 'Dayle',
            'email' => 'dayle@example.com',
            'password' => Hash::make($this->passwords()['dayle@example.com']),
        ]);
        // A row with every nullable column NULL, as a raw insert leaves it.
        DB::table('users')->insert([
            'name' => 'Jeffrey',
            'email' => 'jeffrey@example.com',
            'password' => Hash::make($this->passwords()['jeffrey@example.com']),
        ]);
        $jeffrey = User::where('email', 'jeffrey@example.com')->firstOrFail();
        $this->expectIds([$taylor->id, $abigail->id, $dayle->id, $jeffrey->id], [1, 2, 3, 4], 'users');

        // Posts: timestamps and a json value, a soft-deleted row, and a row
        // with NULL timestamps.
        $first = Post::create(['title' => 'First', 'meta' => ['tags' => ['a', 'b'], 'draft' => false]]);
        $trashed = Post::create(['title' => 'Trashed']);
        $trashed->delete();
        DB::table('posts')->insert(['title' => 'No timestamps']);
        $third = Post::findOrFail(3);
        $this->expectIds([$first->id, $trashed->id, $third->id], [1, 2, 3], 'posts');

        // Notifications, Pennant flags and spatie assignments are written
        // before the morph map exists, so they hold the class name.
        $taylor->notify(new InvoicePaid(42));
        $taylor->notify(new InvoicePaid(43));
        $taylor->notifications()->get()
            ->first(fn ($notification) => $notification->data['invoice_id'] === 43)
            ->markAsRead();
        $abigail->notify(new InvoicePaid(44));

        Feature::activate('new-api');
        Feature::for($taylor)->activate('purple-theme', 'blue');
        Feature::for($taylor)->deactivate('beta');
        Feature::for($abigail)->activate('beta');
        Feature::deactivate('maintenance');

        Permission::create(['name' => 'edit articles']);
        Permission::create(['name' => 'delete articles']);
        Permission::create(['name' => 'publish articles']);
        Role::create(['name' => 'writer'])->givePermissionTo('edit articles');
        Role::create(['name' => 'admin'])->givePermissionTo(['edit articles', 'delete articles', 'publish articles']);
        $taylor->assignRole('writer');
        $abigail->givePermissionTo('delete articles');
        $jeffrey->assignRole('admin');

        // Polymorphic rows: the class name first, then the alias an
        // application writes once it registers a morph map.
        $first->comments()->create(['body' => 'Comment one']);
        $first->comments()->create(['body' => 'Comment two']);
        $first->image()->create(['url' => 'https://example.com/first.png']);
        Relation::morphMap(['post' => Post::class]);
        $first->comments()->create(['body' => 'Comment three']);
        $first->comments()->create(['body' => 'Comment four']);
        $third->comments()->create(['body' => 'Comment five']);
        $third->image()->create(['url' => 'https://example.com/third.png']);

        // The queue, on the database connection.
        ProcessPodcast::dispatch(7);
        ProcessPodcast::dispatch(10)->onQueue('emails');
        Bus::batch([new ProcessPodcast(8), new ProcessPodcast(9)])->name('podcasts')->dispatch();
        FailingJob::dispatch()->onQueue('failing');
        Artisan::call('queue:work', [
            'connection' => 'database',
            '--queue' => 'failing',
            '--once' => true,
            '--tries' => 1,
        ]);
        if (DB::table('failed_jobs')->count() !== 1) {
            throw new RuntimeException('the worker did not write one failed_jobs row: '.Artisan::output());
        }
        // The stack trace in the exception names files under this build's
        // temporary directory. That prefix is replaced with /var/www/html so
        // the committed fixture names no local path; the rest of the text is
        // what Laravel wrote.
        foreach (DB::table('failed_jobs')->get(['id', 'exception']) as $failed) {
            DB::table('failed_jobs')->where('id', $failed->id)->update([
                'exception' => str_replace(base_path(), '/var/www/html', $failed->exception),
            ]);
        }

        $sessions = [
            'user' => $this->writeSession($taylor),
            'guest' => $this->writeSession(null),
        ];
        // Three hours idle: older than a short session lifetime.
        DB::table('sessions')->update(['last_activity' => time() - 3 * 3600]);

        Cache::put('fixture-key', 'fixture-value', 3600);

        return $sessions;
    }

    // Writes one session row through Laravel's Store and database handler,
    // with the skeleton's json serialization. A user session carries the
    // guard's login key, and the handler fills user_id from the guard.
    private function writeSession(?User $user): string
    {
        $guard = Auth::guard('web');
        if ($user !== null) {
            $guard->setUser($user);
        } else {
            $guard->forgetUser();
        }
        $handler = new DatabaseSessionHandler(DB::connection(), config('session.table'), config('session.lifetime'), app());
        $store = new Store(config('session.cookie'), $handler, null, config('session.serialization', 'php'));
        $store->start();
        $store->put('fixture', $user === null ? 'guest' : 'user');
        if ($user !== null) {
            $store->put($guard->getName(), $user->getAuthIdentifier());
        }
        $store->save();
        $guard->forgetUser();

        return $store->getId();
    }

    private function expectIds(array $actual, array $expected, string $table): void
    {
        if ($actual !== $expected) {
            throw new RuntimeException("{$table} ids are ".json_encode($actual).', not '.json_encode($expected));
        }
    }

    // The tables the schema statements create, in creation order.
    private function createdTables(array $schema): array
    {
        $tables = [];
        foreach ($schema as $sql) {
            if (preg_match('/^create table (?:if not exists )?["`]?([A-Za-z0-9_]+)["`]?/i', $sql, $match)) {
                $tables[] = $match[1];
            }
        }

        return array_values(array_unique($tables));
    }

    // Every row of every table as an INSERT statement, parents before
    // children because the tables are in creation order.
    private function dumpRows(array $tables): array
    {
        $connection = DB::connection();
        $grammar = $connection->getQueryGrammar();
        $pdo = $connection->getPdo();
        $rows = [];
        foreach ($tables as $table) {
            $columns = Schema::getColumnListing($table);
            $order = $this->primaryKey($table) ?? $columns;
            $query = DB::table($table);
            foreach ($order as $column) {
                $query->orderBy($column);
            }
            $wrappedColumns = implode(', ', array_map(fn ($column) => $grammar->wrap($column), $columns));
            foreach ($query->get() as $row) {
                $values = [];
                foreach ($columns as $column) {
                    $values[] = $this->literal($pdo, $row->{$column}, "{$table}.{$column}");
                }
                $rows[] = 'insert into '.$grammar->wrapTable($table).' ('.$wrappedColumns.') values ('.implode(', ', $values).')';
            }
            if ($connection->getDriverName() === 'pgsql' && in_array('id', $columns, true) && $this->hasSequence($table)) {
                $rows[] = "select setval(pg_get_serial_sequence('{$table}', 'id'), coalesce(max(id), 1), max(id) is not null) from ".$grammar->wrapTable($table);
            }
        }

        return $rows;
    }

    private function primaryKey(string $table): ?array
    {
        foreach (Schema::getIndexes($table) as $index) {
            if ($index['primary']) {
                return $index['columns'];
            }
        }

        return null;
    }

    private function hasSequence(string $table): bool
    {
        $sequence = DB::selectOne('select pg_get_serial_sequence(?, ?) as name', [$table, 'id']);

        return $sequence !== null && $sequence->name !== null;
    }

    private function literal(PDO $pdo, mixed $value, string $where): string
    {
        return match (true) {
            $value === null => 'NULL',
            is_int($value) => (string) $value,
            is_float($value) => var_export($value, true),
            is_string($value) => $pdo->quote($value),
            default => throw new RuntimeException("unexpected value type ".get_debug_type($value)." in {$where}"),
        };
    }

    // The fixture users with their ids and plaintext passwords.
    private function users(): array
    {
        $users = [];
        foreach ($this->passwords() as $email => $password) {
            $users[$email] = [
                'id' => User::where('email', $email)->value('id'),
                'password' => $password,
            ];
        }

        return $users;
    }

    private function serverVersion(): string
    {
        $connection = DB::connection();
        if ($connection->getDriverName() === 'sqlite') {
            return 'sqlite '.DB::selectOne('select sqlite_version() as v')->v;
        }

        return $connection->getDriverName().' '.$connection->getPdo()->getAttribute(PDO::ATTR_SERVER_VERSION);
    }
}
