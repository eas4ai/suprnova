<?php

// Checks one fixture that generate.sh just wrote. It replays the fixture's
// schema and rows into an empty database, one statement at a time, and
// compares each table's row count with the database Laravel built. Then it
// checks each user's password with PHP's own password_verify and
// password_get_info.
//
// Usage: php verify.php <fixture.json>
// Env: LDB_VERIFY_BUILT_DSN and LDB_VERIFY_REPLAY_DSN (PDO DSNs), and
// LDB_VERIFY_USER and LDB_VERIFY_PASSWORD for MySQL and Postgres. The
// password is read from the environment so it never appears in argv.

function fail(string $message): never
{
    fwrite(STDERR, "verify: {$message}\n");
    exit(1);
}

$path = $argv[1] ?? fail('usage: php verify.php <fixture.json>');
$fixture = json_decode(file_get_contents($path), true, 512, JSON_THROW_ON_ERROR);

$connect = function (string $dsn): PDO {
    $user = getenv('LDB_VERIFY_USER') ?: null;
    $password = getenv('LDB_VERIFY_PASSWORD') ?: null;

    return new PDO($dsn, $user, $password, [PDO::ATTR_ERRMODE => PDO::ERRMODE_EXCEPTION]);
};
$built = $connect(getenv('LDB_VERIFY_BUILT_DSN') ?: fail('LDB_VERIFY_BUILT_DSN is not set'));
$replay = $connect(getenv('LDB_VERIFY_REPLAY_DSN') ?: fail('LDB_VERIFY_REPLAY_DSN is not set'));

foreach (['schema', 'rows'] as $part) {
    foreach ($fixture[$part] as $i => $sql) {
        try {
            $replay->exec($sql);
        } catch (PDOException $e) {
            fail("{$part}[{$i}] failed: ".$e->getMessage()."\n{$sql}");
        }
    }
}

$tables = [];
foreach ($fixture['schema'] as $sql) {
    if (preg_match('/^create table (?:if not exists )?["`]?([A-Za-z0-9_]+)["`]?/i', $sql, $match)) {
        $tables[$match[1]] = true;
    }
}
$quote = $fixture['engine'] === 'mysql' ? fn ($t) => "`{$t}`" : fn ($t) => "\"{$t}\"";
foreach (array_keys($tables) as $table) {
    $expected = (int) $built->query('select count(*) from '.$quote($table))->fetchColumn();
    $actual = (int) $replay->query('select count(*) from '.$quote($table))->fetchColumn();
    if ($expected !== $actual) {
        fail("{$table}: the replay holds {$actual} rows, Laravel wrote {$expected}");
    }
    echo "{$fixture['engine']} {$table}: {$actual}\n";
}

foreach ($fixture['users'] as $email => $user) {
    $statement = $replay->prepare('select password from users where id = ?');
    $statement->execute([$user['id']]);
    $hash = $statement->fetchColumn();
    if ($hash === false) {
        fail("no user row for {$email}");
    }
    if (! str_starts_with($hash, '$2y$')) {
        fail("{$email}: the hash is not \$2y\$");
    }
    if (password_get_info($hash)['algoName'] !== 'bcrypt') {
        fail("{$email}: password_get_info does not report bcrypt");
    }
    if (! password_verify($user['password'], $hash)) {
        fail("{$email}: password_verify refuses the recorded password");
    }
    echo "{$fixture['engine']} {$email}: ".strlen($user['password'])." byte password verifies\n";
}
