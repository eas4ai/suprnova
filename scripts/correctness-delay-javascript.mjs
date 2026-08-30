import { createRequire } from "node:module";
import path from "node:path";

const require = createRequire(import.meta.url);

export function loadTypeScript(repositoryRoot) {
  return require(
    path.join(
      repositoryRoot,
      "browser/node_modules/typescript/lib/typescript.js",
    ),
  );
}

function unwrap(ts, expression) {
  let current = expression;
  while (
    ts.isParenthesizedExpression(current) ||
    ts.isAsExpression(current) ||
    ts.isTypeAssertionExpression(current) ||
    ts.isNonNullExpression(current) ||
    ts.isSatisfiesExpression(current)
  ) {
    current = current.expression;
  }
  return current;
}

function propertyName(ts, expression) {
  const current = unwrap(ts, expression);
  if (ts.isPropertyAccessExpression(current)) return current.name.text;
  if (
    ts.isElementAccessExpression(current) &&
    current.argumentExpression !== undefined &&
    (ts.isStringLiteral(current.argumentExpression) ||
      ts.isNoSubstitutionTemplateLiteral(current.argumentExpression))
  ) {
    return current.argumentExpression.text;
  }
  return null;
}

function rootName(ts, expression) {
  let current = unwrap(ts, expression);
  while (
    ts.isPropertyAccessExpression(current) ||
    ts.isElementAccessExpression(current)
  ) {
    current = unwrap(ts, current.expression);
  }
  return ts.isIdentifier(current) ? current.text : null;
}

class Scope {
  constructor(parent = null) {
    this.parent = parent;
    this.bindings = new Map();
  }

  bind(name, kind) {
    this.bindings.set(name, kind);
  }

  lookup(name) {
    if (this.bindings.has(name)) return this.bindings.get(name);
    if (this.parent !== null) return this.parent.lookup(name);
    if (name === "Promise") return "promise";
    if (name === "setTimeout") return "timeout";
    if (name === "setImmediate") return "turn";
    return null;
  }
}

function expressionKind(ts, expression, scope) {
  const current = unwrap(ts, expression);
  if (ts.isIdentifier(current)) return scope.lookup(current.text);
  const property = propertyName(ts, current);
  if (property === null) return null;
  if (property === "waitForTimeout") return "playwright-timeout";
  const root = rootName(ts, current);
  if (root === "globalThis" || root === "window" || root === "self") {
    if (property === "Promise") return "promise";
    if (property === "setTimeout") return "timeout";
    if (property === "setImmediate") return "turn";
  }
  return null;
}

function bindName(ts, name, initializer, scope, sourceKind = null) {
  if (name === undefined || ts.isOmittedExpression(name)) return;
  if (ts.isIdentifier(name)) {
    scope.bind(
      name.text,
      initializer === undefined
        ? sourceKind
        : expressionKind(ts, initializer, scope),
    );
    return;
  }
  if (!ts.isObjectBindingPattern(name)) {
    for (const element of name.elements)
      bindName(ts, element.name, undefined, scope, null);
    return;
  }
  const initializerRoot =
    initializer === undefined ? null : rootName(ts, initializer);
  const globalObject =
    initializerRoot === "globalThis" ||
    initializerRoot === "window" ||
    initializerRoot === "self";
  for (const element of name.elements) {
    if (element.dotDotDotToken !== undefined) {
      bindName(ts, element.name, undefined, scope, null);
      continue;
    }
    const selected = element.propertyName ?? element.name;
    const selectedName =
      ts.isIdentifier(selected) || ts.isStringLiteral(selected)
        ? selected.text
        : null;
    const kind =
      globalObject && selectedName === "setTimeout"
        ? "timeout"
        : globalObject && selectedName === "setImmediate"
          ? "turn"
          : globalObject && selectedName === "Promise"
            ? "promise"
            : selectedName === "waitForTimeout"
              ? "playwright-timeout"
              : null;
    bindName(ts, element.name, undefined, scope, kind);
  }
}

function bindDeclaration(ts, node, scope) {
  if (ts.isVariableDeclaration(node))
    bindName(ts, node.name, node.initializer, scope);
  if (ts.isImportSpecifier(node)) {
    const imported = node.propertyName?.text ?? node.name.text;
    const moduleName = node.parent.parent.parent.moduleSpecifier.text;
    const timerModule = new Set([
      "node:timers",
      "timers",
      "node:timers/promises",
    ]);
    const kind =
      moduleName === "node:timers/promises" && imported === "setTimeout"
        ? "promise-timeout"
        : timerModule.has(moduleName) && imported === "setTimeout"
          ? "timeout"
          : timerModule.has(moduleName) && imported === "setImmediate"
            ? "turn"
            : null;
    scope.bind(node.name.text, kind);
  }
  if (ts.isImportClause(node) && node.name !== undefined)
    scope.bind(node.name.text, null);
  if (ts.isFunctionDeclaration(node) && node.name !== undefined)
    scope.bind(node.name.text, null);
  if (ts.isClassDeclaration(node) && node.name !== undefined)
    scope.bind(node.name.text, null);
}

function nestedScope(ts, node, parent) {
  return ts.isFunctionLike(node) || ts.isBlock(node) || ts.isCatchClause(node)
    ? new Scope(parent)
    : parent;
}

function containsCall(ts, node, initialScope, expectedKinds) {
  let found = false;
  function visit(current, inherited) {
    if (found) return;
    const scope =
      current === node ? inherited : nestedScope(ts, current, inherited);
    bindDeclaration(ts, current, scope);
    if (ts.isParameter(current))
      bindName(ts, current.name, undefined, scope, null);
    if (
      ts.isCallExpression(current) &&
      expectedKinds.has(expressionKind(ts, current.expression, scope))
    ) {
      found = true;
      return;
    }
    ts.forEachChild(current, (child) => visit(child, scope));
  }
  visit(node, initialScope);
  return found;
}

function isPromiseResolve(ts, node, scope) {
  if (!ts.isCallExpression(node)) return false;
  const expression = unwrap(ts, node.expression);
  if (propertyName(ts, expression) !== "resolve") return false;
  if (
    !ts.isPropertyAccessExpression(expression) &&
    !ts.isElementAccessExpression(expression)
  ) {
    return false;
  }
  return expressionKind(ts, expression.expression, scope) === "promise";
}

function loopContainsPromiseTurn(ts, loop, initialScope) {
  const statements = ts.isBlock(loop.statement)
    ? loop.statement.statements
    : [loop.statement];
  return (
    statements.length > 0 &&
    statements.every((statement) => {
      if (!ts.isExpressionStatement(statement)) return false;
      const expression = unwrap(ts, statement.expression);
      return (
        ts.isAwaitExpression(expression) &&
        isPromiseResolve(ts, unwrap(ts, expression.expression), initialScope)
      );
    })
  );
}

function comments(ts, sourceFile) {
  const ranges = new Map();
  const collect = (range) => {
    if (range !== undefined)
      ranges.set(`${String(range.pos)}:${String(range.end)}`, range);
  };
  function visit(node) {
    for (const range of ts.getLeadingCommentRanges(
      sourceFile.text,
      node.getFullStart(),
    ) ?? []) {
      collect(range);
    }
    for (const range of ts.getTrailingCommentRanges(
      sourceFile.text,
      node.getEnd(),
    ) ?? []) {
      collect(range);
    }
    ts.forEachChild(node, visit);
  }
  visit(sourceFile);
  return [...ranges.values()]
    .sort((left, right) => left.pos - right.pos)
    .map((range) => {
      const raw = sourceFile.text.slice(range.pos, range.end);
      return {
        line: sourceFile.getLineAndCharacterOfPosition(range.pos).line + 1,
        text: raw.slice(
          2,
          range.kind === ts.SyntaxKind.MultiLineCommentTrivia ? -2 : undefined,
        ),
      };
    });
}

export function scanJavaScript(ts, filePath, source) {
  const sourceFile = ts.createSourceFile(
    filePath,
    source,
    ts.ScriptTarget.Latest,
    true,
    filePath.endsWith(".tsx") ? ts.ScriptKind.TSX : ts.ScriptKind.TS,
  );
  const parseDiagnostics = sourceFile.parseDiagnostics ?? [];
  if (parseDiagnostics.length > 0) {
    return {
      comments: comments(ts, sourceFile),
      violations: [
        {
          kind: "parse-error",
          line:
            sourceFile.getLineAndCharacterOfPosition(
              parseDiagnostics[0].start ?? 0,
            ).line + 1,
        },
      ],
    };
  }

  const violations = [];
  const seen = new Set();
  const add = (kind, node) => {
    const line =
      sourceFile.getLineAndCharacterOfPosition(node.getStart(sourceFile)).line +
      1;
    const key = `${kind}:${String(line)}`;
    if (!seen.has(key)) {
      seen.add(key);
      violations.push({ kind, line });
    }
  };

  function visit(node, inherited) {
    const scope = nestedScope(ts, node, inherited);
    bindDeclaration(ts, node, scope);
    if (ts.isParameter(node)) bindName(ts, node.name, undefined, scope, null);
    if (
      ts.isCallExpression(node) &&
      expressionKind(ts, node.expression, scope) === "playwright-timeout"
    ) {
      add("playwright-timeout", node);
    }
    if (
      ts.isCallExpression(node) &&
      expressionKind(ts, node.expression, scope) === "promise-timeout"
    ) {
      add("promise-timeout", node);
    }
    if (
      ts.isNewExpression(node) &&
      expressionKind(ts, node.expression, scope) === "promise"
    ) {
      const executor = node.arguments?.[0];
      if (
        executor !== undefined &&
        containsCall(ts, executor, scope, new Set(["timeout"]))
      ) {
        add("promise-timeout", node);
      }
      if (
        executor !== undefined &&
        containsCall(ts, executor, scope, new Set(["turn"]))
      ) {
        add("promise-turn-wait", node);
      }
    }
    if (
      (ts.isForStatement(node) ||
        ts.isForInStatement(node) ||
        ts.isForOfStatement(node) ||
        ts.isWhileStatement(node) ||
        ts.isDoStatement(node)) &&
      loopContainsPromiseTurn(ts, node, scope)
    ) {
      add("promise-turn-loop", node);
    }
    ts.forEachChild(node, (child) => visit(child, scope));
  }

  visit(sourceFile, new Scope());
  return { comments: comments(ts, sourceFile), violations };
}
