<?php
/**
 * Extract Laravel's public surface from laravel/framework source, as JSON records,
 * and with a second root the surface of the inertiajs/inertia-laravel package.
 *
 * Usage: php extract.php <laravel-root> <out.jsonl> <exclusions.json> [<inertia-laravel-root>]
 *
 * Everything comes from the framework source at the checked-out tag, read
 * through PHP reflection over Composer's classmap, plus the framework's own
 * config files. Anything that cannot be loaded, or that Laravel marks
 * @internal, is recorded in the exclusions file with the reason.
 *
 * inertia-laravel is read the same way, loaded beside the framework it
 * extends: its classes through its composer.json PSR-4 map, its helpers, its
 * Artisan commands and its config file. What its service providers register
 * at run time (macros on Laravel classes, Blade directives and component
 * namespaces, middleware aliases) is read by registering the providers its
 * composer.json names on a bare application and comparing the registries
 * before and after. Every record carries `package`, the Composer package it
 * comes from, and its file relative to that package's root.
 */

[$_, $root, $outPath, $exclPath] = $argv;
$root = realpath($root);
$inertiaRoot = isset($argv[4]) ? realpath($argv[4]) : null;
if (isset($argv[4]) && $inertiaRoot === false) {
    fwrite(STDERR, "no inertia-laravel checkout at {$argv[4]}\n");
    exit(1);
}
require $root . '/vendor/autoload.php';
$classmap = require $root . '/vendor/composer/autoload_classmap.php';
$src = $root . '/src/Illuminate/';

$records = [];
/** Facade classes by name: their @method lines are records, their @see lines link them. */
$facades = [];
$excluded = ['unloadable' => [], 'internal' => [], 'blade_compiler_passes' => []];
/** The Composer package the records being emitted come from. */
$package = 'laravel/framework';
/** Package roots, and the marker each is replaced with in values (see portable()). */
$roots = [$root => '<laravel>'];
if ($inertiaRoot !== null) {
    $roots[$inertiaRoot] = '<inertia-laravel>';
}

/** A file relative to the root of the package it belongs to. */
function rel(string $file): string {
    global $roots;
    foreach (array_keys($roots) as $r) {
        if (str_starts_with($file, $r . '/')) {
            return substr($file, strlen($r) + 1);
        }
    }
    throw new RuntimeException("$file is under no package root");
}

/** Replace each clone's absolute path, so records don't depend on where it sits on disk. */
function portable(string $s): string {
    global $roots;
    return strtr($s, $roots);
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
    if (str_starts_with($class, 'Illuminate\\')) {
        return $parts[1] ?? '';
    }
    // A package's own sub-namespace (Inertia\Ssr\Gateway is Ssr), else the package's (Inertia).
    return count($parts) > 2 ? $parts[1] : $parts[0];
}

function emit(array $r): void {
    global $records, $package;
    if (($r['details'] ?? null) === []) {
        $r['details'] = new stdClass;  // an empty object, not a JSON list
    }
    $r['package'] = $package;
    ksort($r);
    $records[] = $r;
}

/**
 * Every Macroable class's registered macros, by the class that owns the static table
 * (a subclass shares its parent's), so a provider's additions are a before/after diff.
 */
function macro_tables(): array {
    $tables = [];
    foreach (get_declared_classes() as $c) {
        if (!in_array(\Illuminate\Support\Traits\Macroable::class, class_uses_recursive($c), true)) {
            continue;
        }
        $prop = new ReflectionProperty($c, 'macros');
        $tables[$prop->getDeclaringClass()->getName()] ??= $prop->getValue();
    }
    ksort($tables);
    return $tables;
}

// ---- Classes, interfaces, traits, enums and their members -------------------
$classes = array_filter(array_keys($classmap), fn ($c) =>
    str_starts_with($c, 'Illuminate\\') && str_starts_with(realpath($classmap[$c]) ?: '', $src));
sort($classes);
/** Every class to read, each with the package it belongs to. */
$allClasses = array_map(fn ($c) => ['laravel/framework', $c], $classes);
/** inertia-laravel's helper files, and the service providers it asks Laravel to register. */
$inertiaHelpers = $inertiaProviders = [];
$inertiaClasses = [];
if ($inertiaRoot !== null) {
    $composer = json_decode(file_get_contents("$inertiaRoot/composer.json"), true, flags: JSON_THROW_ON_ERROR);
    $psr4 = $composer['autoload']['psr-4'] ?? [];
    if (!$psr4) {
        fwrite(STDERR, "inertia-laravel's composer.json declares no PSR-4 autoload\n");
        exit(1);
    }
    foreach ($psr4 as $prefix => $dir) {
        $base = realpath("$inertiaRoot/$dir");
        spl_autoload_register(function ($class) use ($prefix, $base) {
            if (str_starts_with($class, $prefix)) {
                $file = $base . '/' . str_replace('\\', '/', substr($class, strlen($prefix))) . '.php';
                if (is_file($file)) {
                    require $file;
                }
            }
        });
        $files = new RecursiveIteratorIterator(new RecursiveDirectoryIterator($base, FilesystemIterator::SKIP_DOTS));
        foreach ($files as $f) {
            if ($f->getExtension() === 'php') {
                $inertiaClasses[] = $prefix . str_replace('/', '\\', substr($f->getPathname(), strlen($base) + 1, -4));
            }
        }
    }
    sort($inertiaClasses);
    foreach ($composer['autoload']['files'] ?? [] as $f) {
        $inertiaHelpers[] = realpath("$inertiaRoot/$f");
        require_once "$inertiaRoot/$f";
    }
    $inertiaProviders = $composer['extra']['laravel']['providers'] ?? [];
    foreach ($inertiaClasses as $c) {
        $allClasses[] = ['inertiajs/inertia-laravel', $c];
    }
}

foreach ($allClasses as [$package, $class]) {
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
    $isFacade = str_starts_with($class, 'Illuminate\\Support\\Facades\\')
        || $rc->isSubclassOf(\Illuminate\Support\Facades\Facade::class);
    if ($isFacade) {
        $facades[$class] = true;
    }
    if ($isFacade && ($doc = $rc->getDocComment())) {
        $docStart = $rc->getStartLine() - substr_count($doc, "\n") - 1;
        foreach (explode("\n", $doc) as $i => $line) {
            if (preg_match('/@method\s+(static\s+)?(.+?)\s+([A-Za-z_][A-Za-z0-9_]*)\((.*)\)\s*$/', $line, $m)
                && !($rc->hasMethod($m[3]) && $rc->getMethod($m[3])->getDeclaringClass()->getName() === $class)) {
                $sig = trim(($m[1] ? 'static ' : '') . "$m[2] $m[3]($m[4])");
                emit([
                    'id' => "$class::$m[3]", 'kind' => 'facade method', 'family' => 'laravel-api',
                    'component' => component_of($class), 'namespace' => $rc->getNamespaceName(), 'parent' => $class,
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
    if ($r['kind'] !== 'facade method' && isset($facades[$r['id']]) && $r['parent'] === null) {
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
    $fnFile = $rf->getFileName() ?: '';
    if (str_starts_with($fnFile, $src)) {
        $package = 'laravel/framework';
        $component = component_of('Illuminate\\' . basename(dirname($fnFile)));
    } elseif (in_array($fnFile, $inertiaHelpers, true)) {
        $package = 'inertiajs/inertia-laravel';
        $component = 'Inertia';
    } else {
        continue;
    }
    $sig = method_sig($rf);
    emit([
        'id' => 'helper ' . $rf->getName(), 'kind' => 'helper', 'family' => 'laravel-helpers',
        'component' => $component,
        'namespace' => '', 'parent' => null, 'file' => rel($rf->getFileName()), 'line' => $rf->getStartLine(),
        'deprecated' => doc_flags($rf->getDocComment())['deprecated'], 'details' => ['signature' => $sig],
        'sig_hash' => digest($sig), 'body_hash' => digest(source_span($rf->getFileName(), $rf->getStartLine(), $rf->getEndLine())),
    ]);
}

// ---- Artisan commands ----------------------------------------------------------
foreach ($allClasses as [$package, $class]) {
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

$package = 'laravel/framework';

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
$configFiles = array_map(fn ($f) => ['laravel/framework', $f], glob($root . '/config/*.php'));
if ($inertiaRoot !== null) {
    foreach (glob($inertiaRoot . '/config/*.php') as $f) {
        $configFiles[] = ['inertiajs/inertia-laravel', $f];
    }
}
foreach ($configFiles as [$package, $cfg]) {
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
                      'default' => isset($e[2]) ? trim($e[2][0]) : null, 'package' => $package];
    }
}

// ---- What inertia-laravel's service providers register ------------------------------
// Laravel registers the providers a package's composer.json names (extra.laravel.providers).
// Registering them on a bare application, with the config, files and Blade compiler bound,
// runs their register() methods; boot() never runs, so no route or command is installed.
// The records are what changed: macros on Macroable classes, Blade directives and component
// namespaces, and middleware aliases.
if ($inertiaRoot !== null) {
    $package = 'inertiajs/inertia-laravel';
    $app = new \Illuminate\Foundation\Application($root);
    $app->instance('config', new \Illuminate\Config\Repository([]));
    $app->singleton('files', fn () => new \Illuminate\Filesystem\Filesystem);
    $app->singleton('blade.compiler', fn ($app) => new \Illuminate\View\Compilers\BladeCompiler($app['files'], sys_get_temp_dir()));
    $aliasesBefore = $app['router']->getMiddleware();
    $macrosBefore = macro_tables();
    foreach ($inertiaProviders as $provider) {
        $app->register($provider);
    }
    $compiler = $app->make('blade.compiler');  // runs the providers' afterResolving callbacks

    foreach (macro_tables() as $owner => $macros) {
        foreach ($macros as $name => $macro) {
            if (array_key_exists($name, $macrosBefore[$owner] ?? [])) {
                continue;
            }
            $fn = new ReflectionFunction(Closure::fromCallable($macro));
            $sig = 'function ' . $name . '(' . implode(', ', array_map('param_str', $fn->getParameters())) . ')'
                . ($fn->hasReturnType() ? ': ' . type_str($fn->getReturnType()) : '');
            emit([
                'id' => "$owner::$name", 'kind' => 'macro', 'family' => 'laravel-api', 'component' => component_of($owner),
                'namespace' => (new ReflectionClass($owner))->getNamespaceName(), 'parent' => null,
                'file' => rel($fn->getFileName()), 'line' => $fn->getStartLine(), 'deprecated' => false,
                'details' => ['on' => $owner, 'signature' => $sig], 'sig_hash' => digest($sig),
                'body_hash' => digest(source_span($fn->getFileName(), $fn->getStartLine(), $fn->getEndLine())),
            ]);
        }
    }

    foreach ($compiler->getCustomDirectives() as $name => $handler) {
        $fn = is_array($handler) ? new ReflectionMethod($handler[0], $handler[1])
            : new ReflectionFunction(Closure::fromCallable($handler));
        $method = $fn instanceof ReflectionMethod ? $fn->getDeclaringClass()->getName() . '::' . $fn->getName() : null;
        emit([
            'id' => '@' . $name, 'kind' => 'blade directive', 'family' => 'laravel-blade', 'component' => 'View',
            'namespace' => $fn instanceof ReflectionMethod ? $fn->getDeclaringClass()->getNamespaceName() : '',
            'parent' => null, 'file' => rel($fn->getFileName()), 'line' => $fn->getStartLine(), 'deprecated' => false,
            'details' => array_filter(['method' => $method]), 'sig_hash' => digest(method_sig($fn)),
            'body_hash' => digest(source_span($fn->getFileName(), $fn->getStartLine(), $fn->getEndLine())),
        ]);
    }

    // <x-prefix::name> resolves to the namespace's class Str::studly(name), each '.' a sub-namespace.
    foreach ($compiler->getClassComponentNamespaces() as $prefix => $namespace) {
        foreach ($inertiaClasses as $class) {
            if (!str_starts_with($class, $namespace . '\\') || !is_subclass_of($class, \Illuminate\View\Component::class)) {
                continue;
            }
            $rc = new ReflectionClass($class);
            $tag = implode('.', array_map(fn ($seg) => \Illuminate\Support\Str::kebab($seg),
                explode('\\', substr($class, strlen($namespace) + 1))));
            $ctor = $rc->getConstructor();
            emit([
                'id' => "<x-$prefix::$tag>", 'kind' => 'blade component', 'family' => 'laravel-blade', 'component' => 'View',
                'namespace' => $rc->getNamespaceName(), 'parent' => null, 'file' => rel($rc->getFileName()),
                'line' => $rc->getStartLine(), 'deprecated' => false, 'details' => ['class' => $class],
                'sig_hash' => digest("$tag " . ($ctor ? method_sig($ctor) : '')), 'body_hash' => null,
            ]);
        }
    }

    foreach (array_diff_key($app['router']->getMiddleware(), $aliasesBefore) as $alias => $class) {
        $rc = new ReflectionClass($class);
        emit([
            'id' => "middleware $alias", 'kind' => 'middleware alias', 'family' => 'laravel-api',
            'component' => component_of($class), 'namespace' => $rc->getNamespaceName(), 'parent' => null,
            'file' => rel($rc->getFileName()), 'line' => $rc->getStartLine(), 'deprecated' => false,
            'details' => ['class' => $class], 'sig_hash' => digest("$alias=$class"), 'body_hash' => null,
        ]);
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
    $package = $reads[0]['package'];
    if (count(array_unique(array_column($reads, 'package'))) > 1) {
        fwrite(STDERR, "env $envName is read by the config of more than one package\n");
        exit(1);
    }
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
$byPackage = array_count_values(array_column($records, 'package'));
ksort($byPackage);
echo json_encode(['records' => count($records), 'by_kind' => $byKind, 'by_package' => $byPackage,
                  'unloadable' => count((array) $excluded['unloadable']), 'internal' => count($excluded['internal'])]), "\n";
