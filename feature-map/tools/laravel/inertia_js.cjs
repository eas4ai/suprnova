'use strict'
/**
 * Extract Inertia.js's public surface from the inertiajs/inertia source, as JSON records.
 *
 * Usage: node inertia_js.cjs <typescript-module-dir> <inertia-root> <out.jsonl>
 *
 * Two kinds of record, both read with TypeScript's own parser (never by matching
 * text):
 *
 * - Every name each package exports, per entry point its package.json `exports`
 *   map declares (`@inertiajs/core`, `@inertiajs/core/server`, `@inertiajs/react`,
 *   ...). Re-exports are followed to the declaration, across packages too, and
 *   the record cites the file and line that declares the name. A Svelte
 *   component is cited at its `.svelte` file.
 * - Every `X-Inertia*` header the client sends or reads (`header X-Inertia-Version`),
 *   from the `@inertiajs/core` source: the protocol a server adapter answers. A
 *   header is sent where it is a key of a headers object or the target of an
 *   assignment, and read where it is the argument of `getHeader` or `hasHeader`.
 *
 * Anything the extractor cannot place stops the run with the file and line:
 * an export form it does not know, a re-export it cannot resolve, or a header
 * literal in a position that is neither a send nor a read. Records carry no
 * machine paths: files are relative to the inertia root.
 */
const crypto = require('crypto')
const fs = require('fs')
const path = require('path')

const [tsDir, rootArg, outPath] = process.argv.slice(2)
if (!tsDir || !rootArg || !outPath) {
  console.error('usage: node inertia_js.cjs <typescript-module-dir> <inertia-root> <out.jsonl>')
  process.exit(2)
}
const ts = require(path.resolve(tsDir))
const root = fs.realpathSync(rootArg)
const PACKAGE = 'inertiajs/inertia'

/** Stop the run with the reason; nothing has been written yet. */
function fail(message) {
  console.error(`inertia_js: ${message}`)
  process.exit(1)
}

function rel(file) {
  return path.relative(root, file).split(path.sep).join('/')
}

/** The same digest extract.php computes: whitespace runs collapsed, sha256, 16 hex digits. */
function digest(text) {
  const t = text.trim().replace(/\s+/g, ' ')
  return t === '' ? null : crypto.createHash('sha256').update(t).digest('hex').slice(0, 16)
}

/** Drop comment lines, so a doc comment edit is not an API change (as extract.php does). */
function withoutCommentLines(text) {
  return text
    .split('\n')
    .filter((l) => !/^\s*(\/\/|\*|\/\*)/.test(l))
    .join('\n')
}

const sources = new Map()
function parse(file) {
  if (!sources.has(file)) {
    const kind = file.endsWith('.tsx') ? ts.ScriptKind.TSX : ts.ScriptKind.TS
    sources.set(file, ts.createSourceFile(file, fs.readFileSync(file, 'utf8'), ts.ScriptTarget.Latest, true, kind))
  }
  return sources.get(file)
}

function lineOf(node) {
  const sf = node.getSourceFile()
  return sf.getLineAndCharacterOfPosition(node.getStart(sf)).line + 1
}

// ---- Packages and their entry points --------------------------------------------------
const packages = fs
  .readdirSync(path.join(root, 'packages'))
  .filter((d) => fs.existsSync(path.join(root, 'packages', d, 'package.json')))
  .sort()
  .map((dir) => {
    const pkgDir = path.join(root, 'packages', dir)
    const manifest = JSON.parse(fs.readFileSync(path.join(pkgDir, 'package.json'), 'utf8'))
    if (manifest.private) {
      return null
    }
    const entries = {}
    for (const [sub, target] of Object.entries(manifest.exports || {})) {
      const built = typeof target === 'string' ? target : target.import || target.svelte || target.default
      const m = /^\.\/dist\/(.+)\.js$/.exec(built || '')
      if (!m) {
        fail(`${manifest.name}: export ${sub} maps to ${JSON.stringify(target)}, not ./dist/<file>.js`)
      }
      const src = path.join(pkgDir, 'src', `${m[1]}.ts`)
      if (!fs.existsSync(src)) {
        fail(`${manifest.name}: export ${sub} has no source at ${rel(src)}`)
      }
      entries[sub === '.' ? manifest.name : `${manifest.name}/${sub.replace(/^\.\//, '')}`] = src
    }
    return { name: manifest.name, version: manifest.version, dir: pkgDir, short: dir, entries }
  })
  .filter(Boolean)
const entryFiles = new Map()
for (const p of packages) {
  for (const [module, file] of Object.entries(p.entries)) {
    entryFiles.set(module, file)
  }
}

function packageOf(file) {
  return packages.find((p) => file.startsWith(p.dir + path.sep))
}

/** A module specifier, resolved from `from` to a file: TypeScript, or a `.svelte` component. */
function resolveModule(from, spec) {
  if (spec.startsWith('@inertiajs/')) {
    const file = entryFiles.get(spec)
    if (!file) {
      fail(`${rel(from)}: ${spec} is no entry point any package.json exports`)
    }
    return file
  }
  if (!spec.startsWith('.')) {
    return null // a dependency outside the Inertia packages
  }
  const base = path.resolve(path.dirname(from), spec)
  for (const candidate of [`${base}.ts`, `${base}.tsx`, path.join(base, 'index.ts'), path.join(base, 'index.tsx')]) {
    if (fs.existsSync(candidate)) {
      return candidate
    }
  }
  if (/\.(svelte|vue)$/.test(base) && fs.existsSync(base)) {
    return base
  }
  fail(`${rel(from)}: cannot resolve ${spec}`)
}

// ---- Declarations -----------------------------------------------------------------------
const hasModifier = (node, kind) => (ts.canHaveModifiers(node) ? ts.getModifiers(node) || [] : []).some((m) => m.kind === kind)

function declKind(node) {
  if (ts.isFunctionDeclaration(node)) return 'ts function'
  if (ts.isClassDeclaration(node)) return 'ts class'
  if (ts.isInterfaceDeclaration(node)) return 'ts interface'
  if (ts.isTypeAliasDeclaration(node)) return 'ts type'
  if (ts.isEnumDeclaration(node)) return 'ts enum'
  if (ts.isModuleDeclaration(node)) return 'ts namespace'
  if (ts.isVariableDeclaration(node)) {
    const init = node.initializer
    return init && (ts.isArrowFunction(init) || ts.isFunctionExpression(init)) ? 'ts function' : 'ts const'
  }
  if (ts.isExportAssignment(node)) {
    const e = node.expression
    return ts.isArrowFunction(e) || ts.isFunctionExpression(e) ? 'ts function' : 'ts value'
  }
  fail(`${rel(node.getSourceFile().fileName)}:${lineOf(node)}: no kind for ${ts.SyntaxKind[node.kind]}`)
}

/** The text a caller depends on: a function's parameters and return type, not its body. */
function signatureText(node) {
  const sf = node.getSourceFile()
  const text = (n) => n.getText(sf)
  const head = (fn, prefix) => {
    const end = fn.body ? fn.body.getStart(sf) : fn.getEnd()
    return prefix + sf.text.slice(fn.getStart(sf), end).trim()
  }
  if (ts.isFunctionDeclaration(node)) return head(node, '')
  if (ts.isVariableDeclaration(node)) {
    const init = node.initializer
    if (init && (ts.isArrowFunction(init) || ts.isFunctionExpression(init))) {
      return head(init, `${text(node.name)}${node.type ? ': ' + text(node.type) : ''} = `)
    }
    return text(node)
  }
  if (ts.isExportAssignment(node)) {
    const e = node.expression
    return ts.isArrowFunction(e) || ts.isFunctionExpression(e) ? head(e, 'default = ') : text(node)
  }
  if (ts.isClassDeclaration(node)) {
    // The class's non-private members, each without its body.
    const members = node.members
      .filter((m) => !hasModifier(m, ts.SyntaxKind.PrivateKeyword) && !(m.name && ts.isPrivateIdentifier(m.name)))
      .map((m) => (m.body ? sf.text.slice(m.getStart(sf), m.body.getStart(sf)).trim() : text(m)))
    const header = sf.text.slice(node.getStart(sf), node.members.pos).trim()
    return [header, ...members].join('\n')
  }
  return text(node)
}

function isDeprecated(node) {
  return ts.getJSDocTags(node).some((t) => t.tagName.text === 'deprecated')
}

/** Whether a binding name (an identifier or a destructuring pattern) binds `name`. */
function bindsName(binding, name) {
  if (ts.isIdentifier(binding)) return binding.text === name
  return binding.elements.some((el) => !ts.isOmittedExpression(el) && bindsName(el.name, name))
}

/** The declarations a file makes of `name` at its top level (overloads and merged declarations included). */
function localDeclarations(sf, name) {
  const found = []
  for (const st of sf.statements) {
    if (ts.isVariableStatement(st)) {
      for (const d of st.declarationList.declarations) {
        if (bindsName(d.name, name)) found.push(d)
      }
    } else if (
      (ts.isFunctionDeclaration(st) || ts.isClassDeclaration(st) || ts.isInterfaceDeclaration(st) ||
        ts.isTypeAliasDeclaration(st) || ts.isEnumDeclaration(st) || ts.isModuleDeclaration(st)) &&
      st.name && st.name.text === name
    ) {
      found.push(st)
    }
  }
  return found
}

function target(decls) {
  return { file: decls[0].getSourceFile().fileName, decls }
}

function component(file) {
  return { file, decls: null }
}

/** What a top-level `name` in a file is: a local declaration or an imported binding. */
function resolveLocal(sf, name) {
  const decls = localDeclarations(sf, name)
  if (decls.length) {
    return target(decls)
  }
  for (const st of sf.statements) {
    if (!ts.isImportDeclaration(st) || !st.importClause) continue
    const spec = st.moduleSpecifier.text
    const clause = st.importClause
    let imported = null
    if (clause.name && clause.name.text === name) {
      imported = 'default'
    }
    const bindings = clause.namedBindings
    if (bindings && ts.isNamedImports(bindings)) {
      for (const el of bindings.elements) {
        if (el.name.text === name) imported = (el.propertyName || el.name).text
      }
    }
    if (bindings && ts.isNamespaceImport(bindings) && bindings.name.text === name) {
      fail(`${rel(sf.fileName)}:${lineOf(st)}: re-export of the namespace import ${name}`)
    }
    if (imported !== null) {
      const file = resolveModule(sf.fileName, spec)
      if (file === null) {
        fail(`${rel(sf.fileName)}:${lineOf(st)}: ${name} re-exports ${spec}, outside the Inertia packages`)
      }
      return exportOf(file, imported, `${rel(sf.fileName)}:${lineOf(st)}`)
    }
  }
  fail(`${rel(sf.fileName)}: no declaration or import of ${name}`)
}

function exportOf(file, name, site) {
  if (!/\.tsx?$/.test(file)) {
    if (name !== 'default') fail(`${site}: ${rel(file)} has only a default export, not ${name}`)
    return component(file)
  }
  const found = moduleExports(file).get(name)
  if (!found) {
    fail(`${site}: ${rel(file)} does not export ${name}`)
  }
  return found
}

const exportCache = new Map()
/** Every name a module exports, mapped to where it is declared. */
function moduleExports(file) {
  if (exportCache.has(file)) {
    const cached = exportCache.get(file)
    if (cached === null) fail(`${rel(file)}: circular export *`)
    return cached
  }
  exportCache.set(file, null)
  const sf = parse(file)
  const out = new Map()
  // Two statements exporting one declaration (an overloaded function) are one export.
  const same = (a, b) => a.file === b.file && (a.decls === null ? b.decls === null : b.decls !== null && a.decls[0] === b.decls[0])
  const add = (name, t, node) => {
    if (out.has(name) && !same(out.get(name), t)) {
      fail(`${rel(file)}:${lineOf(node)}: ${name} is exported twice`)
    }
    out.set(name, t)
  }
  for (const st of sf.statements) {
    const site = `${rel(file)}:${lineOf(st)}`
    if (ts.isExportDeclaration(st)) {
      const spec = st.moduleSpecifier ? st.moduleSpecifier.text : null
      const from = spec === null ? null : resolveModule(file, spec)
      if (spec !== null && from === null) {
        fail(`${site}: re-exports ${spec}, outside the Inertia packages`)
      }
      if (!st.exportClause) {
        for (const [name, t] of moduleExports(from)) {
          if (name !== 'default') add(name, t, st)
        }
      } else if (ts.isNamespaceExport(st.exportClause)) {
        fail(`${site}: export * as ${st.exportClause.name.text} is not a form this extractor knows`)
      } else {
        for (const el of st.exportClause.elements) {
          const local = (el.propertyName || el.name).text
          add(el.name.text, from === null ? resolveLocal(sf, local) : exportOf(from, local, site), st)
        }
      }
    } else if (ts.isExportAssignment(st)) {
      if (st.isExportEquals) fail(`${site}: export = is not a form this extractor knows`)
      const e = st.expression
      add('default', ts.isIdentifier(e) ? resolveLocal(sf, e.text) : target([st]), st)
    } else if (hasModifier(st, ts.SyntaxKind.ExportKeyword)) {
      if (hasModifier(st, ts.SyntaxKind.DefaultKeyword)) {
        const prior = out.get('default')
        const overload = (d) => ts.isFunctionDeclaration(d) && ts.isFunctionDeclaration(st) && d.name && st.name &&
          d.name.text === st.name.text
        if (prior && prior.decls && prior.decls.every(overload)) {
          // An overload of the default-exported function: one export, every signature.
          out.set('default', target([...prior.decls, st]))
        } else {
          add('default', target([st]), st)
        }
      } else if (ts.isVariableStatement(st)) {
        for (const d of st.declarationList.declarations) {
          if (!ts.isIdentifier(d.name)) fail(`${site}: a destructured export is not a form this extractor knows`)
          add(d.name.text, target(localDeclarations(sf, d.name.text)), st)
        }
      } else if (st.name) {
        add(st.name.text, target(localDeclarations(sf, st.name.text)), st)
      } else {
        fail(`${site}: an exported ${ts.SyntaxKind[st.kind]} this extractor does not know`)
      }
    }
  }
  exportCache.set(file, out)
  return out
}

// ---- Records ----------------------------------------------------------------------------
const records = []
for (const p of packages) {
  for (const [module, entry] of Object.entries(p.entries)) {
    for (const [name, t] of moduleExports(entry)) {
      const declaring = packageOf(t.file)
      const details = { module, name }
      if (declaring && declaring !== p) {
        details.declared_in = declaring.name
      }
      let kind, line, sig, body, deprecated
      if (t.decls === null) {
        // A Svelte component: its <script> block is what a page passes props to.
        const text = fs.readFileSync(t.file, 'utf8')
        const script = /<script\b[^>]*>([\s\S]*?)<\/script>/.exec(text)
        kind = 'svelte component'
        line = 1
        sig = script ? script[1] : ''
        body = text
        deprecated = false
      } else {
        kind = declKind(t.decls[0])
        line = lineOf(t.decls[0])
        sig = t.decls.map(signatureText).join('\n')
        body = t.decls.map((d) => d.getText(d.getSourceFile())).join('\n')
        deprecated = t.decls.some(isDeprecated)
        if (kind === 'ts function') {
          details.signature = sig.replace(/\s+/g, ' ')
        }
      }
      records.push({
        body_hash: digest(withoutCommentLines(body)),
        component: p.short,
        deprecated,
        details,
        family: 'inertia-js-api',
        file: rel(t.file),
        id: `${module} ${name}`,
        kind,
        line,
        namespace: module,
        package: PACKAGE,
        parent: null,
        sig_hash: digest(withoutCommentLines(sig)),
      })
    }
  }
}

// ---- Protocol headers the client sends and reads -------------------------------------------
const HEADER = /^x-inertia(-[a-z]+)*$/i
const canonical = (h) => h.split('-').map((s) => s[0].toUpperCase() + s.slice(1).toLowerCase()).join('-')

/** Whether a use of a header name sends it, reads it, or neither (null). */
function useOf(node) {
  const parent = node.parent
  if (ts.isPropertyAssignment(parent) && parent.name === node) return 'sent'
  if (ts.isComputedPropertyName(parent) && ts.isPropertyAssignment(parent.parent)) return 'sent'
  if (
    ts.isElementAccessExpression(parent) && parent.argumentExpression === node &&
    ts.isBinaryExpression(parent.parent) && parent.parent.left === parent &&
    parent.parent.operatorToken.kind === ts.SyntaxKind.EqualsToken
  ) {
    return 'sent'
  }
  if (ts.isCallExpression(parent) && parent.arguments.includes(node)) {
    const callee = parent.expression
    const name = ts.isPropertyAccessExpression(callee) ? callee.name.text : ts.isIdentifier(callee) ? callee.text : ''
    if (name === 'getHeader' || name === 'hasHeader') return 'read'
  }
  return null
}

const core = packages.find((p) => p.name === '@inertiajs/core')
if (!core) fail('no @inertiajs/core package')
const headers = new Map()
function sourceFiles(dir) {
  return fs
    .readdirSync(dir, { withFileTypes: true })
    .sort((a, b) => (a.name < b.name ? -1 : 1))
    .flatMap((d) => {
      const full = path.join(dir, d.name)
      if (d.isDirectory()) return sourceFiles(full)
      return /\.tsx?$/.test(d.name) && !/\.d\.ts$/.test(d.name) ? [full] : []
    })
}
for (const file of sourceFiles(path.join(core.dir, 'src'))) {
  const sf = parse(file)
  const visit = (node) => {
    if (ts.isStringLiteralLike(node) && HEADER.test(node.text)) {
      const name = canonical(node.text)
      const sites = []
      const parent = node.parent
      if (ts.isVariableDeclaration(parent) && parent.initializer === node && ts.isIdentifier(parent.name)) {
        // A constant holding the header name: each use of the constant is a site.
        const constant = parent.name.text
        const uses = []
        const find = (n) => {
          if (ts.isIdentifier(n) && n.text === constant && n !== parent.name) uses.push(n)
          ts.forEachChild(n, find)
        }
        find(sf)
        if (!uses.length) fail(`${rel(file)}:${lineOf(node)}: the header constant ${constant} is never used`)
        sites.push(...uses)
      } else {
        sites.push(node)
      }
      for (const site of sites) {
        const use = useOf(site)
        if (use === null) {
          fail(`${rel(file)}:${lineOf(site)}: ${name} is neither sent nor read here`)
        }
        if (!headers.has(name)) headers.set(name, { sent_at: [], read_at: [] })
        headers.get(name)[`${use}_at`].push(`${rel(file)}:${lineOf(site)}`)
      }
    }
    ts.forEachChild(node, visit)
  }
  visit(sf)
}
for (const [name, h] of headers) {
  h.sent_at.sort()
  h.read_at.sort()
  const [first] = [...h.sent_at, ...h.read_at].sort()
  const [file, line] = first.split(':')
  records.push({
    body_hash: null,
    component: core.short,
    deprecated: false,
    details: h,
    family: 'inertia-protocol',
    file,
    id: `header ${name}`,
    kind: 'protocol header',
    line: Number(line),
    namespace: core.name,
    package: PACKAGE,
    parent: null,
    sig_hash: digest(`${name} sent:${h.sent_at.length > 0} read:${h.read_at.length > 0}`),
  })
}

// ---- Write ------------------------------------------------------------------------------------
records.sort((a, b) => (a.id < b.id ? -1 : a.id > b.id ? 1 : 0))
const dupes = records.filter((r, i) => i > 0 && records[i - 1].id === r.id).map((r) => r.id)
if (dupes.length) fail(`duplicate ids: ${dupes.slice(0, 10).join(', ')}`)
fs.writeFileSync(outPath, records.map((r) => JSON.stringify(r) + '\n').join(''))
const byKind = {}
for (const r of records) byKind[r.kind] = (byKind[r.kind] || 0) + 1
console.log(
  JSON.stringify({
    records: records.length,
    by_kind: byKind,
    packages: Object.fromEntries(packages.map((p) => [p.name, p.version])),
  }),
)
