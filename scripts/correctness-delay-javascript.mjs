import { createRequire } from "node:module";
import path from "node:path";

const require = createRequire(import.meta.url);
const delayModules = new Set([
  "node:timers",
  "node:timers/promises",
  "timers",
  "timers/promises",
]);
const delayPrimitives = new Set([
  "setImmediate",
  "setInterval",
  "setTimeout",
  "waitForTimeout",
]);
const dynamicTextMarker = "__SUPRNOVA_DYNAMIC_TEXT_EXPRESSION__";
const inertScriptTypes = new Set(["application/json", "application/ld+json"]);

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

function literalText(ts, node) {
  const current = node === undefined ? undefined : unwrap(ts, node);
  return current !== undefined &&
    (ts.isStringLiteral(current) || ts.isNoSubstitutionTemplateLiteral(current))
    ? current.text
    : null;
}

function selectedName(ts, node) {
  if (ts.isPropertyAccessExpression(node)) return node.name.text;
  if (ts.isElementAccessExpression(node))
    return literalText(ts, node.argumentExpression);
  return null;
}

function propertyNameText(ts, node) {
  if (node === undefined) return null;
  if (ts.isComputedPropertyName(node)) return literalText(ts, node.expression);
  if (ts.isIdentifier(node)) return node.text;
  return literalText(ts, node);
}

function moduleReference(ts, node) {
  if (
    (ts.isImportDeclaration(node) || ts.isExportDeclaration(node)) &&
    node.moduleSpecifier !== undefined
  ) {
    return literalText(ts, node.moduleSpecifier);
  }
  if (
    ts.isImportEqualsDeclaration(node) &&
    ts.isExternalModuleReference(node.moduleReference)
  ) {
    return literalText(ts, node.moduleReference.expression);
  }
  if (ts.isCallExpression(node) && node.arguments.length === 1) {
    const expression = unwrap(ts, node.expression);
    if (
      (ts.isIdentifier(expression) && expression.text === "require") ||
      expression.kind === ts.SyntaxKind.ImportKeyword
    ) {
      return literalText(ts, node.arguments[0]);
    }
  }
  return null;
}

function isPropertyNamePosition(ts, node) {
  const parent = node.parent;
  return (
    (ts.isPropertyAccessExpression(parent) && parent.name === node) ||
    (ts.isPropertyAssignment(parent) && parent.name === node) ||
    (ts.isMethodDeclaration(parent) && parent.name === node) ||
    (ts.isPropertyDeclaration(parent) && parent.name === node) ||
    (ts.isPropertySignature(parent) && parent.name === node) ||
    (ts.isMethodSignature(parent) && parent.name === node) ||
    (ts.isBindingElement(parent) && parent.propertyName === node) ||
    (ts.isImportSpecifier(parent) && parent.propertyName === node) ||
    (ts.isExportSpecifier(parent) && parent.propertyName === node)
  );
}

function isDestructuringAssignmentProperty(ts, node) {
  let current = node;
  while (current.parent !== undefined) {
    const parent = current.parent;
    if (ts.isParenthesizedExpression(parent) && parent.expression === current) {
      current = parent;
      continue;
    }
    if (
      (ts.isObjectLiteralExpression(parent) ||
        ts.isArrayLiteralExpression(parent)) &&
      current.parent === parent
    ) {
      current = parent;
      continue;
    }
    if (ts.isPropertyAssignment(parent) && parent.initializer === current) {
      current = parent;
      continue;
    }
    return (
      ts.isBinaryExpression(parent) &&
      parent.left === current &&
      parent.operatorToken.kind === ts.SyntaxKind.EqualsToken
    );
  }
  return false;
}

function reconstructedTextAssembly(ts, sourceFile, node) {
  const current = unwrap(ts, node);
  if (
    ts.isStringLiteral(current) ||
    ts.isNoSubstitutionTemplateLiteral(current)
  ) {
    return {
      dynamic: false,
      start: current.getStart(sourceFile) + 1,
      text: current.text,
    };
  }
  if (ts.isTemplateExpression(current)) {
    let dynamic = false;
    let text = current.head.text;
    for (const span of current.templateSpans) {
      const expression = reconstructedTextAssembly(
        ts,
        sourceFile,
        span.expression,
      );
      if (expression === null) {
        const raw = sourceFile.text.slice(
          span.expression.getStart(sourceFile),
          span.expression.getEnd(),
        );
        dynamic = true;
        text += `${dynamicTextMarker}${"\n".repeat(raw.split("\n").length - 1)}`;
      } else {
        dynamic ||= expression.dynamic;
        text += expression.text;
      }
      text += span.literal.text;
    }
    return { dynamic, start: current.getStart(sourceFile) + 1, text };
  }
  if (
    !ts.isBinaryExpression(current) ||
    current.operatorToken.kind !== ts.SyntaxKind.PlusToken
  ) {
    return null;
  }
  const left = reconstructedTextAssembly(ts, sourceFile, current.left);
  const right = reconstructedTextAssembly(ts, sourceFile, current.right);
  if (left === null && right === null) return null;
  return {
    dynamic:
      left === null || right === null || left.dynamic || right.dynamic,
    start: current.getStart(sourceFile),
    text: `${left?.text ?? dynamicTextMarker}${right?.text ?? dynamicTextMarker}`,
  };
}

function isNestedTextAssembly(ts, node) {
  const parent = node.parent;
  if (parent === undefined) return false;
  return (
    (ts.isBinaryExpression(parent) &&
      parent.operatorToken.kind === ts.SyntaxKind.PlusToken) ||
    (ts.isTemplateSpan(parent) && parent.expression === node)
  );
}

function attributeValue(attributes, name) {
  const expression = new RegExp(
    `(?:^|\\s)${name}\\s*=\\s*(?:"([^"]*)"|'([^']*)'|([^\\s"'=<>\u0060]+))`,
    "iu",
  );
  const match = expression.exec(attributes);
  return match === null ? null : (match[1] ?? match[2] ?? match[3] ?? "");
}

function executableInlineScripts(ts, sourceFile) {
  const scripts = [];
  const violations = [];
  const expression = /<script\b([^>]*)>([\s\S]*?)<\/script\s*>/giu;
  function visit(node) {
    const assembly = isNestedTextAssembly(ts, node)
      ? null
      : reconstructedTextAssembly(ts, sourceFile, node);
    if (assembly !== null) {
      const withoutDynamicText = assembly.text.replaceAll(dynamicTextMarker, "");
      const hasPotentialScript = /<script\b/iu.test(withoutDynamicText);
      let matchedScript = false;
      for (const match of assembly.text.matchAll(expression)) {
        matchedScript = true;
        const attributes = match[1] ?? "";
        const body = match[2] ?? "";
        const bodyOffset = (match.index ?? 0) + match[0].indexOf(body);
        const beforeBody = assembly.text.slice(0, bodyOffset);
        const line =
          sourceFile.getLineAndCharacterOfPosition(assembly.start).line +
          1 +
          (beforeBody.split("\n").length - 1);
        const dynamic = match[0].includes(dynamicTextMarker);
        const source = attributeValue(attributes, "src");
        if (source !== null && !source.includes(dynamicTextMarker)) continue;

        const type = attributeValue(attributes, "type");
        if (
          type !== null &&
          !type.includes(dynamicTextMarker) &&
          inertScriptTypes.has(type.trim().toLowerCase())
        ) {
          continue;
        }
        if (dynamic) violations.push({ kind: "inline-script-assembly", line });
        scripts.push({
          line,
          source: body.replaceAll(dynamicTextMarker, "undefined"),
        });
      }
      if (hasPotentialScript && !matchedScript) {
        violations.push({
          kind: "inline-script-assembly",
          line:
            sourceFile.getLineAndCharacterOfPosition(assembly.start).line + 1,
        });
      }
    }
    ts.forEachChild(node, visit);
  }
  visit(sourceFile);
  return { scripts, violations };
}

function isPromiseResolveCall(ts, expression) {
  const current = unwrap(ts, expression);
  if (!ts.isCallExpression(current)) return false;
  const callee = unwrap(ts, current.expression);
  if (selectedName(ts, callee) !== "resolve") return false;
  if (
    !ts.isPropertyAccessExpression(callee) &&
    !ts.isElementAccessExpression(callee)
  ) {
    return false;
  }
  const owner = unwrap(ts, callee.expression);
  if (ts.isIdentifier(owner)) return owner.text === "Promise";
  return selectedName(ts, owner) === "Promise";
}

function isAwaitedPromiseTurn(ts, statement) {
  if (!ts.isExpressionStatement(statement)) return false;
  const expression = unwrap(ts, statement.expression);
  return (
    ts.isAwaitExpression(expression) &&
    isPromiseResolveCall(ts, expression.expression)
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

export function scanJavaScript(ts, filePath, source, scanEmbedded = true) {
  const sourceFile = ts.createSourceFile(
    filePath,
    source,
    ts.ScriptTarget.Latest,
    true,
    filePath.endsWith(".tsx") || filePath.endsWith(".jsx")
      ? ts.ScriptKind.TSX
      : ts.ScriptKind.TS,
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

  const sourceComments = comments(ts, sourceFile);
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
  const addAtLine = (kind, line) => {
    const key = `${kind}:${String(line)}`;
    if (!seen.has(key)) {
      seen.add(key);
      violations.push({ kind, line });
    }
  };

  function visit(node) {
    const referencedModule = moduleReference(ts, node);
    if (referencedModule !== null && delayModules.has(referencedModule)) {
      add("delay-module-reference", node);
    }

    const property = selectedName(ts, node);
    if (property !== null && delayPrimitives.has(property)) {
      add("delay-primitive-reference", node);
    }
    if (
      ts.isCallExpression(node) &&
      selectedName(ts, unwrap(ts, node.expression)) === "get" &&
      literalText(ts, node.arguments[1]) !== null &&
      delayPrimitives.has(literalText(ts, node.arguments[1]))
    ) {
      add("delay-primitive-reference", node);
    }
    if (
      ts.isBindingElement(node) &&
      node.propertyName !== undefined &&
      delayPrimitives.has(propertyNameText(ts, node.propertyName))
    ) {
      add("delay-primitive-reference", node);
    }
    if (
      ts.isPropertyAssignment(node) &&
      isDestructuringAssignmentProperty(ts, node) &&
      delayPrimitives.has(propertyNameText(ts, node.name))
    ) {
      add("delay-primitive-reference", node);
    }
    if (
      ts.isIdentifier(node) &&
      delayPrimitives.has(node.text) &&
      !isPropertyNamePosition(ts, node)
    ) {
      add("delay-primitive-reference", node);
    }
    if (ts.isBlock(node) || ts.isSourceFile(node)) {
      let consecutiveTurns = [];
      for (const statement of node.statements) {
        if (isAwaitedPromiseTurn(ts, statement)) {
          consecutiveTurns.push(statement);
        } else {
          if (consecutiveTurns.length > 1)
            add("promise-turn-loop", consecutiveTurns[0]);
          consecutiveTurns = [];
        }
      }
      if (consecutiveTurns.length > 1)
        add("promise-turn-loop", consecutiveTurns[0]);
    }
    if (
      (ts.isForStatement(node) ||
        ts.isForInStatement(node) ||
        ts.isForOfStatement(node) ||
        ts.isWhileStatement(node) ||
        ts.isDoStatement(node)) &&
      (ts.isBlock(node.statement)
        ? node.statement.statements.some((statement) =>
            isAwaitedPromiseTurn(ts, statement),
          )
        : isAwaitedPromiseTurn(ts, node.statement))
    ) {
      add("promise-turn-loop", node);
    }
    ts.forEachChild(node, visit);
  }

  visit(sourceFile);
  if (scanEmbedded) {
    const embeddedScripts = executableInlineScripts(ts, sourceFile);
    for (const violation of embeddedScripts.violations) {
      addAtLine(violation.kind, violation.line);
    }
    for (const embedded of embeddedScripts.scripts) {
      const scanned = scanJavaScript(ts, filePath, embedded.source, false);
      sourceComments.push(
        ...scanned.comments.map((comment) => ({
          ...comment,
          line: embedded.line + comment.line - 1,
        })),
      );
      for (const violation of scanned.violations) {
        const line = embedded.line + violation.line - 1;
        addAtLine(violation.kind, line);
      }
    }
  }
  return {
    comments: sourceComments.sort((left, right) => left.line - right.line),
    violations,
  };
}
