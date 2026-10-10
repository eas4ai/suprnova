<?php
/**
 * Extract Laravel's public surface from laravel/framework source, as JSON records.
 *
 * Usage: php extract.php <laravel-root> <out.jsonl> <exclusions.json>
 *
 * Everything comes from the framework source at the checked-out tag, read
 * through PHP reflection over Composer's classmap, plus the framework's own
 * config files. Anything that cannot be loaded, or that Laravel marks
 * @internal, is recorded in the exclusions file with the reason.
 */

[$_, $root, $outPath, $exclPath] = $argv;
$root = realpath($root);
require $root . '/vendor/autoload.php';
$classmap = require $root . '/vendor/composer/autoload_classmap.php';
$src = $root . '/src/Illuminate/';

$records = [];
$excluded = ['unloadable' => [], 'internal' => [], 'blade_compiler_passes' => []];

function rel(string $file): string {
    global $root;
    return ltrim(substr($file, strlen($root)), '/');
}

/** Replace the clone's absolute path, so records don't depend on where it sits on disk. */
function portable(string $s): string {
    global $root;
    return str_replace($root, '<laravel>', $s);
}

function digest(string $text): ?string {
    $text = preg_replace('/\s+/', ' ', trim($text));
    return $text === '' ? null : substr(hash('sha256', $text), 0, 16);
}

function source_span(string $file, int $start, int $end): string {
    static $cache = [];
    $cache[$file] ??= file($file);
    $lines = array_slice($cache[$file], $start - 1, $end - $start + 1);
    // Drop comment lines so a docblock edit is not an API change.
    $lines = array_filter($lines, fn ($l) => !preg_match('#^\s*(//|\*|/\*)#', $l));
    return implode('', $lines);
}

function type_str(?ReflectionType $t): string {
    return $t === null ? '' : (string) $t;
}

function param_str(ReflectionParameter $p): string {
    $s = type_str($p->getType());
    $s .= ($s !== '' ? ' ' : '') . ($p->isPassedByReference() ? '&' : '') . ($p->isVariadic() ? '...' : '') . '$' . $p->getName();
    if ($p->isDefaultValueAvailable()) {
        try {
            $s .= ' = ' . ($p->isDefaultValueConstant() ? $p->getDefaultValueConstantName()
                : portable(preg_replace('/\s+/', ' ', var_export($p->getDefaultValue(), true))));
        } catch (Throwable $e) {
            $s .= ' = ?';
        }
    }
    return $s;
}

function method_sig(ReflectionMethod|ReflectionFunction $m): string {
    $mods = $m instanceof ReflectionMethod ? implode(' ', Reflection::getModifierNames($m->getModifiers())) . ' ' : '';
    $ret = $m->hasReturnType() ? ': ' . type_str($m->getReturnType()) : '';
    return trim($mods . 'function ' . $m->getName() . '(' . implode(', ', array_map('param_str', $m->getParameters())) . ')' . $ret);
}

function doc_flags(string|false $doc): array {
    return [
        'internal' => $doc !== false && str_contains($doc, '@internal'),
        'deprecated' => $doc !== false && str_contains($doc, '@deprecated'),
    ];
}

function component_of(string $class): string {
    $parts = explode('\\', $class);
    return $parts[1] ?? '';
}

function emit(array $r): void {
    global $records;
    if (($r['details'] ?? null) === []) {
        $r['details'] = new stdClass;  // an empty object, not a JSON list
    }
    ksort($r);
    $records[] = $r;
}

// ---- Classes, interfaces, traits, enums and their members -------------------
$classes = array_filter(array_keys($classmap), fn ($c) =>
    str_starts_with($c, 'Illuminate\\') && str_starts_with(realpath($classmap[$c]) ?: '', $src));
sort($classes);

foreach ($classes as $class) {
    try {
        $rc = new ReflectionClass($class);
    } catch (Throwable $e) {
        $excluded['unloadable'][$class] = get_class($e) . ': ' . $e->getMessage();
        continue;
    }
    $flags = doc_flags($rc->getDocComment());
    if ($flags['internal']) {
        $excluded['internal'][] = $class;
        continue;
    }
    $kind = $rc->isInterface() ? 'interface' : ($rc->isTrait() ? 'trait' : ($rc->isEnum() ? 'enum'
        : ($rc->isAbstract() ? 'abstract class' : ($rc->isFinal() ? 'final class' : 'class'))));
    $file = $rc->getFileName();
    $extendable = !$rc->isFinal() && !$rc->isEnum();
    $parent = $rc->getParentClass();
    $details = [
        'extends' => $parent ? $parent->getName() : null,
        'implements' => array_values(array_filter($rc->getInterfaceNames(), fn ($i) => str_starts_with($i, 'Illuminate\\'))),
        'uses_traits' => array_values($rc->getTraitNames()),
    ];
    if ($rc->isEnum()) {
        $details['cases'] = array_map(fn ($c) => $c->getName(), $rc->getReflectionConstants());
    }
    $header = source_span($file, $rc->getStartLine(), $rc->getStartLine());
    emit([
        'id' => $class, 'kind' => $kind, 'family' => 'laravel-api', 'component' => component_of($class),
        'namespace' => $rc->getNamespaceName(), 'parent' => null, 'file' => rel($file), 'line' => $rc->getStartLine(),
        'deprecated' => $flags['deprecated'], 'details' => array_filter($details, fn ($v) => $v !== null && $v !== []),
        'sig_hash' => digest($header . json_encode($details)),
        'body_hash' => digest(source_span($file, $rc->getStartLine(), $rc->getEndLine())),
    ]);

    // Members declared in this file only: inherited members live on their parent,
    // trait members on the trait. Protected members count for extendable types,
    // since subclasses are how Laravel exposes them (Model::$fillable, Command::handle).
    $visible = fn ($m) => $m->isPublic() || ($extendable && $m->isProtected());
    foreach ($rc->getMethods() as $m) {
        if ($m->getFileName() !== $file || $m->getDeclaringClass()->getName() !== $class || !$visible($m)) {
            continue;
        }
        $mf = doc_flags($m->getDocComment());
        if ($mf['internal']) {
            $excluded['internal'][] = "$class::{$m->getName()}";
            continue;
        }
        $sig = method_sig($m);
        emit([
            'id' => "$class::{$m->getName()}", 'kind' => 'method', 'family' => 'laravel-api',
            'component' => component_of($class), 'namespace' => $rc->getNamespaceName(), 'parent' => $class,
            'file' => rel($file), 'line' => $m->getStartLine(), 'deprecated' => $mf['deprecated'],
            'details' => ['signature' => $sig, 'visibility' => $m->isPublic() ? 'public' : 'protected',
                          'static' => $m->isStatic()],
            'sig_hash' => digest($sig),
            'body_hash' => $m->getStartLine() ? digest(source_span($file, $m->getStartLine(), $m->getEndLine())) : null,
        ]);
    }
    foreach ($rc->getProperties() as $p) {
        if ($p->getDeclaringClass()->getName() !== $class || !$visible($p) || $p->isPromoted()) {
            continue;
        }
        // A property declared by a used trait is reported by the trait.
        $fromTrait = false;
        foreach ($rc->getTraits() as $t) {
            if ($t->hasProperty($p->getName())) {
                $fromTrait = true;
            }
        }
        if ($fromTrait) {
            continue;
        }
        $default = $p->hasDefaultValue() ? portable(preg_replace('/\s+/', ' ', var_export($p->getDefaultValue(), true))) : null;
        $sig = trim(implode(' ', Reflection::getModifierNames($p->getModifiers())) . ' ' . type_str($p->getType()) . ' $' . $p->getName());
        emit([
            'id' => "$class::\${$p->getName()}", 'kind' => 'property', 'family' => 'laravel-api',
            'component' => component_of($class), 'namespace' => $rc->getNamespaceName(), 'parent' => $class,
            'file' => rel($file), 'line' => null, 'deprecated' => doc_flags($p->getDocComment())['deprecated'],
            'details' => array_filter(['signature' => $sig, 'default' => $default], fn ($v) => $v !== null),
            'sig_hash' => digest($sig . '=' . $default), 'body_hash' => null,
        ]);
    }
    foreach ($rc->getReflectionConstants() as $c) {
        if ($rc->isEnum() || $c->getDeclaringClass()->getName() !== $class || !($c->isPublic() || ($extendable && $c->isProtected()))) {
            continue;
        }
        $val = portable(preg_replace('/\s+/', ' ', var_export($c->getValue(), true)));
        emit([
            'id' => "$class::{$c->getName()}", 'kind' => 'constant', 'family' => 'laravel-api',
            'component' => component_of($class), 'namespace' => $rc->getNamespaceName(), 'parent' => $class,
            'file' => rel($file), 'line' => null, 'deprecated' => false,
            'details' => ['value' => strlen($val) > 200 ? substr($val, 0, 200) . '...' : $val],
            'sig_hash' => digest($c->getName() . '=' . $val), 'body_hash' => null,
        ]);
    }

    // Facades declare their static API in @method lines: that is the documented surface.
    if (str_starts_with($class, 'Illuminate\\Support\\Facades\\') && ($doc = $rc->getDocComment())) {
        $docStart = $rc->getStartLine() - substr_count($doc, "\n") - 1;
        foreach (explode("\n", $doc) as $i => $line) {
            if (preg_match('/@method\s+(static\s+)?(.+?)\s+([A-Za-z_][A-Za-z0-9_]*)\((.*)\)\s*$/', $line, $m)
                && !($rc->hasMethod($m[3]) && $rc->getMethod($m[3])->getDeclaringClass()->getName() === $class)) {
                $sig = trim(($m[1] ? 'static ' : '') . "$m[2] $m[3]($m[4])");
                emit([
                    'id' => "$class::$m[3]", 'kind' => 'facade method', 'family' => 'laravel-api',
                    'component' => 'Support', 'namespace' => $rc->getNamespaceName(), 'parent' => $class,
                    'file' => rel($file), 'line' => $docStart + $i + 1, 'deprecated' => false,
                    'details' => ['signature' => $sig], 'sig_hash' => digest($sig), 'body_hash' => null,
                ]);
            } elseif (preg_match('/@see\s+\\\\?(\S+)/', $line, $m)) {
                // recorded on the facade itself below
            }
        }
    }
}

// Facade -> the classes its @see lines name, for linking facade methods to implementations.
foreach ($records as &$r) {
    if ($r['kind'] !== 'facade method' && str_starts_with($r['id'], 'Illuminate\\Support\\Facades\\') && $r['parent'] === null) {
        $doc = (new ReflectionClass($r['id']))->getDocComment() ?: '';
        preg_match_all('/@see\s+\\\\?(\S+)/', $doc, $m);
        if ($m[1]) {
            $r['details']['see'] = $m[1];
        }
    }
}
unset($r);

// ---- Global helpers ----------------------------------------------------------
foreach (get_defined_functions()['user'] as $fn) {
    $rf = new ReflectionFunction($fn);
    if (!str_starts_with($rf->getFileName() ?: '', $src)) {
        continue;
    }
    $sig = method_sig($rf);
    emit([
        'id' => 'helper ' . $rf->getName(), 'kind' => 'helper', 'family' => 'laravel-helpers',
        'component' => component_of('Illuminate\\' . basename(dirname($rf->getFileName()))),
        'namespace' => '', 'parent' => null, 'file' => rel($rf->getFileName()), 'line' => $rf->getStartLine(),
        'deprecated' => doc_flags($rf->getDocComment())['deprecated'], 'details' => ['signature' => $sig],
        'sig_hash' => digest($sig), 'body_hash' => digest(source_span($rf->getFileName(), $rf->getStartLine(), $rf->getEndLine())),
    ]);
}

// ---- Artisan commands ----------------------------------------------------------
foreach ($classes as $class) {
    try {
        $rc = new ReflectionClass($class);
    } catch (Throwable $e) {
        continue;
    }
    if ($rc->isAbstract() || !$rc->isSubclassOf(\Illuminate\Console\Command::class)) {
        continue;
    }
    $defaults = $rc->getDefaultProperties();
    $attr = $rc->getAttributes(\Symfony\Component\Console\Attribute\AsCommand::class)[0] ?? null;
    $attrArgs = $attr ? $attr->getArguments() : [];
    $signature = $defaults['signature'] ?? null;
    $name = $attrArgs['name'] ?? $attrArgs[0] ?? $defaults['name'] ?? null;
    $args = $opts = [];
    if ($signature) {
        [$sigName, $arguments, $options] = \Illuminate\Console\Parser::parse($signature);
        $name ??= $sigName;
        foreach ($arguments as $a) {
            $args[] = ['name' => $a->getName(), 'required' => $a->isRequired(), 'description' => $a->getDescription()];
        }
        foreach ($options as $o) {
            $opts[] = ['name' => '--' . $o->getName(), 'shortcut' => $o->getShortcut(), 'description' => $o->getDescription()];
        }
    }
    if (!$name) {
        $excluded['unnamed_commands'][] = $class;
        continue;
    }
    $details = array_filter([
        'class' => $class, 'description' => $defaults['description'] ?? ($attrArgs['description'] ?? $attrArgs[1] ?? null),
        'signature' => $signature, 'arguments' => $args, 'options' => $opts,
        'hidden' => (bool) ($defaults['hidden'] ?? ($attrArgs['hidden'] ?? false)),
    ], fn ($v) => $v !== null && $v !== []);
    emit([
        'id' => 'artisan ' . $name, 'kind' => 'artisan command', 'family' => 'laravel-artisan',
        'component' => component_of($class), 'namespace' => $rc->getNamespaceName(), 'parent' => null,
        'file' => rel($rc->getFileName()), 'line' => $rc->getStartLine(), 'deprecated' => false,
        'details' => $details, 'sig_hash' => digest(($signature ?? $name) . json_encode($opts)), 'body_hash' => null,
    ]);
}

// ---- Blade directives -------------------------------------------------------------
// BladeCompiler dispatches `@name` to `compile` . ucfirst(name) with one argument. The
// compiler's own passes are the methods its pipeline reaches from compileString: the
// $compilers list, the echo methods, and every $this->compileX() call inside them.
// Everything else it defines as compile* is a directive, unless it needs more than one
// argument (dispatch passes exactly one).
$blade = new ReflectionClass(\Illuminate\View\Compilers\BladeCompiler::class);
$bladeInstance = new \Illuminate\View\Compilers\BladeCompiler(new \Illuminate\Filesystem\Filesystem, sys_get_temp_dir());
$echoMethods = (fn () => $this->getEchoMethods())->call($bladeInstance);
$seeds = array_merge(['compileString'], array_map(fn ($c) => "compile$c", $blade->getDefaultProperties()['compilers']), $echoMethods);
$passes = [];
while ($seeds) {
    $name = array_pop($seeds);
    if (isset($passes[$name]) || !$blade->hasMethod($name)) {
        continue;
    }
    $m = $blade->getMethod($name);
    $passes[$name] = true;
    $body = source_span($m->getFileName(), $m->getStartLine(), $m->getEndLine());
    preg_match_all('/\$this->(compile[A-Z]\w*)\(/', $body, $calls);
    array_push($seeds, ...$calls[1]);
}
foreach ($blade->getMethods() as $m) {
    if (!preg_match('/^compile([A-Z][A-Za-z]*)$/', $m->getName(), $mm)) {
        continue;
    }
    if (isset($passes[$m->getName()])) {
        $excluded['blade_compiler_passes'][] = $m->getName() . ' (compile pipeline pass)';
        continue;
    }
    if ($m->getNumberOfRequiredParameters() > 1) {
        $excluded['blade_compiler_passes'][] = $m->getName() . ' (needs ' . $m->getNumberOfRequiredParameters() . ' arguments; @-dispatch passes one)';
        continue;
    }
    $name = '@' . lcfirst($mm[1]);
    $sig = method_sig($m);
    emit([
        'id' => $name, 'kind' => 'blade directive', 'family' => 'laravel-blade', 'component' => 'View',
        'namespace' => 'Illuminate\\View\\Compilers', 'parent' => null, 'file' => rel($m->getFileName()),
        'line' => $m->getStartLine(), 'deprecated' => false,
        'details' => ['method' => $m->getDeclaringClass()->getName() . '::' . $m->getName()],
        'sig_hash' => digest($sig), 'body_hash' => digest(source_span($m->getFileName(), $m->getStartLine(), $m->getEndLine())),
    ]);
}

// ---- Validation rules --------------------------------------------------------------
$va = new ReflectionClass(\Illuminate\Validation\Concerns\ValidatesAttributes::class);
foreach ($va->getMethods() as $m) {
    if (!preg_match('/^validate([A-Z][A-Za-z0-9]*)$/', $m->getName(), $mm)) {
        continue;
    }
    $rule = \Illuminate\Support\Str::snake($mm[1]);
    emit([
        'id' => 'rule ' . $rule, 'kind' => 'validation rule', 'family' => 'laravel-validation', 'component' => 'Validation',
        'namespace' => 'Illuminate\\Validation', 'parent' => null, 'file' => rel($m->getFileName()),
        'line' => $m->getStartLine(), 'deprecated' => doc_flags($m->getDocComment())['deprecated'],
        'details' => ['method' => 'ValidatesAttributes::' . $m->getName(), 'signature' => method_sig($m)],
        'sig_hash' => digest(method_sig($m)),
        'body_hash' => digest(source_span($m->getFileName(), $m->getStartLine(), $m->getEndLine())),
    ]);
}

// ---- Config keys and the env vars they read ----------------------------------------
// Config files call path helpers (storage_path() and friends), which resolve through the
// application instance; a bare Application rooted at the clone is enough. Nothing boots.
new \Illuminate\Foundation\Application($root);
foreach (glob($root . '/config/*.php') as $cfg) {
    $name = basename($cfg, '.php');
    $text = file_get_contents($cfg);
    $values = require $cfg;
    $walk = function ($arr, $prefix) use (&$walk, $name, $cfg, $text) {
        foreach ($arr as $k => $v) {
            $key = portable($prefix === '' ? "$k" : "$prefix.$k");
            if (is_array($v) && $v !== [] && array_keys($v) !== range(0, count($v) - 1)) {
                $walk($v, $key);
                continue;
            }
            $leaf = is_int($k) ? null : $k;
            $line = null;
            if ($leaf !== null && preg_match("/^.*'" . preg_quote((string) $leaf, '/') . "'\s*=>/m", $text, $mm, PREG_OFFSET_CAPTURE)) {
                $line = substr_count(substr($text, 0, $mm[0][1]), "\n") + 1;
            }
            $val = portable(preg_replace('/\s+/', ' ', var_export($v, true)));
            emit([
                'id' => "config $name.$key", 'kind' => 'config key', 'family' => 'laravel-config', 'component' => ucfirst($name),
                'namespace' => '', 'parent' => null, 'file' => rel($cfg), 'line' => $line, 'deprecated' => false,
                'details' => ['default' => strlen($val) > 200 ? substr($val, 0, 200) . '...' : $val],
                'sig_hash' => digest("$key=$val"), 'body_hash' => null,
            ]);
        }
    };
    $walk($values, '');
    preg_match_all("/env\(\s*'([A-Z0-9_]+)'(?:\s*,\s*([^)]*))?\)/", $text, $envs, PREG_OFFSET_CAPTURE | PREG_SET_ORDER);
    foreach ($envs as $e) {
        $line = substr_count(substr($text, 0, $e[0][1]), "\n") + 1;
        $envName = $e[1][0];
        $records[] = ['__env' => true, 'name' => $envName, 'file' => rel($cfg), 'line' => $line,
                      'default' => isset($e[2]) ? trim($e[2][0]) : null];
    }
}
// One record per env var, with every config line that reads it.
$envs = [];
foreach ($records as $i => $r) {
    if (!empty($r['__env'])) {
        $envs[$r['name']][] = $r;
        unset($records[$i]);
    }
}
ksort($envs);
foreach ($envs as $envName => $reads) {
    emit([
        'id' => "env $envName", 'kind' => 'env var', 'family' => 'laravel-config', 'component' => 'Config',
        'namespace' => '', 'parent' => null, 'file' => $reads[0]['file'], 'line' => $reads[0]['line'], 'deprecated' => false,
        'details' => ['read_by' => array_map(fn ($r) => "{$r['file']}:{$r['line']}", $reads),
                      'defaults' => array_values(array_unique(array_filter(array_map(fn ($r) => $r['default'], $reads))))],
        'sig_hash' => digest(json_encode(array_map(fn ($r) => $r['default'], $reads))), 'body_hash' => null,
    ]);
}

// ---- Write --------------------------------------------------------------------------
$records = array_values($records);
usort($records, fn ($a, $b) => strcmp($a['id'], $b['id']));
$ids = array_count_values(array_column($records, 'id'));
$dupes = array_keys(array_filter($ids, fn ($n) => $n > 1));
if ($dupes) {
    fwrite(STDERR, 'duplicate ids: ' . implode(', ', array_slice($dupes, 0, 10)) . "\n");
    exit(1);
}
$out = fopen($outPath, 'w');
foreach ($records as $r) {
    fwrite($out, json_encode($r, JSON_UNESCAPED_SLASHES | JSON_UNESCAPED_UNICODE) . "\n");
}
fclose($out);
$excluded['unloadable'] = (object) $excluded['unloadable'];
file_put_contents($exclPath, json_encode($excluded, JSON_PRETTY_PRINT | JSON_UNESCAPED_SLASHES) . "\n");
$byKind = array_count_values(array_column($records, 'kind'));
ksort($byKind);
echo json_encode(['records' => count($records), 'by_kind' => $byKind,
                  'unloadable' => count((array) $excluded['unloadable']), 'internal' => count($excluded['internal'])]), "\n";
