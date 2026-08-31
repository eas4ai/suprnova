import { spawnSync } from "node:child_process";

function requiredArray(value, description) {
  if (!Array.isArray(value)) {
    throw new Error(`cargo metadata has invalid ${description}`);
  }
  return value;
}

function requiredString(value, description) {
  if (typeof value !== "string" || value.length === 0) {
    throw new Error(`cargo metadata has invalid ${description}`);
  }
  return value;
}

export function resolveGitWorkspaceRoot(directory, spawn = spawnSync) {
  const result = spawn(
    "git",
    ["-C", directory, "rev-parse", "--show-toplevel"],
    { encoding: "utf8" },
  );

  if (result.error !== undefined) {
    const detail =
      result.error instanceof Error
        ? result.error.message
        : String(result.error);
    throw new Error(`cannot spawn git in ${directory}: ${detail}`, {
      cause: result.error,
    });
  }
  if (result.status !== 0) {
    const stderr =
      typeof result.stderr === "string" ? result.stderr.trim() : "";
    throw new Error(
      `git rev-parse failed in ${directory} with status ${String(result.status)}${stderr.length > 0 ? `: ${stderr}` : ""}`,
    );
  }
  if (typeof result.stdout !== "string" || result.stdout.trim().length === 0) {
    throw new Error(`git rev-parse in ${directory} returned no workspace root`);
  }
  return result.stdout.trim();
}

export function loadLockedCargoMetadata(directory, spawn = spawnSync) {
  const result = spawn(
    "rtk",
    ["cargo", "metadata", "--locked", "--format-version", "1"],
    {
      cwd: directory,
      encoding: "utf8",
      maxBuffer: 64 * 1024 * 1024,
    },
  );

  if (result.error !== undefined) {
    const detail =
      result.error instanceof Error
        ? result.error.message
        : String(result.error);
    throw new Error(`cannot spawn cargo metadata in ${directory}: ${detail}`, {
      cause: result.error,
    });
  }
  if (result.status !== 0) {
    const stderr =
      typeof result.stderr === "string" ? result.stderr.trim() : "";
    throw new Error(
      `cargo metadata failed in ${directory} with status ${String(result.status)}${stderr.length > 0 ? `: ${stderr}` : ""}`,
    );
  }
  if (typeof result.stdout !== "string") {
    throw new Error(`cargo metadata in ${directory} returned no stdout`);
  }

  try {
    return JSON.parse(result.stdout);
  } catch (error) {
    throw new Error(`cargo metadata in ${directory} returned invalid JSON`, {
      cause: error,
    });
  }
}

export function cargoDependencyClosure(metadata, rootPackageNames) {
  if (typeof metadata !== "object" || metadata === null) {
    throw new Error("cargo metadata is not an object");
  }
  const packages = requiredArray(metadata.packages, "packages");
  const workspaceMembers = requiredArray(
    metadata.workspace_members,
    "workspace_members",
  );
  if (typeof metadata.resolve !== "object" || metadata.resolve === null) {
    throw new Error("cargo metadata has invalid resolve graph");
  }
  const nodes = requiredArray(metadata.resolve.nodes, "resolve nodes");
  const requestedRoots = requiredArray(rootPackageNames, "root package names");

  const packagesById = new Map();
  for (const dependency of packages) {
    if (typeof dependency !== "object" || dependency === null) {
      throw new Error("cargo metadata contains an invalid package");
    }
    const id = requiredString(dependency.id, "package id");
    requiredString(dependency.name, `package name for ${id}`);
    if (packagesById.has(id)) {
      throw new Error(`cargo metadata contains duplicate package ${id}`);
    }
    packagesById.set(id, dependency);
  }

  const nodesById = new Map();
  for (const node of nodes) {
    if (typeof node !== "object" || node === null) {
      throw new Error("cargo metadata contains an invalid resolve node");
    }
    const id = requiredString(node.id, "resolve node id");
    const dependencies = requiredArray(
      node.dependencies,
      `dependencies for resolve node ${id}`,
    );
    for (const dependencyId of dependencies) {
      requiredString(dependencyId, `dependency id for resolve node ${id}`);
    }
    if (nodesById.has(id)) {
      throw new Error(`cargo metadata contains duplicate resolve node ${id}`);
    }
    nodesById.set(id, node);
  }

  const workspacePackages = workspaceMembers.map((id) => {
    requiredString(id, "workspace member id");
    const dependency = packagesById.get(id);
    if (dependency === undefined) {
      throw new Error(`workspace package ${id} is absent from Cargo packages`);
    }
    return dependency;
  });

  const pending = [];
  const seenRootNames = new Set();
  for (const rootName of requestedRoots) {
    requiredString(rootName, "root package name");
    if (seenRootNames.has(rootName)) {
      throw new Error(`duplicate Cargo root package ${rootName}`);
    }
    seenRootNames.add(rootName);
    const matches = workspacePackages.filter(
      (dependency) => dependency.name === rootName,
    );
    if (matches.length !== 1) {
      throw new Error(
        `cargo metadata contains ${String(matches.length)} workspace packages named ${rootName}`,
      );
    }
    pending.push(matches[0].id);
  }
  if (pending.length === 0) {
    throw new Error("no Cargo root packages were requested");
  }

  const reachable = new Set();
  while (pending.length > 0) {
    const id = pending.pop();
    if (reachable.has(id)) continue;
    const dependency = packagesById.get(id);
    if (dependency === undefined) {
      throw new Error(`dependency package ${id} is absent from Cargo packages`);
    }
    const node = nodesById.get(id);
    if (node === undefined) {
      throw new Error(
        `resolve node for dependency package ${dependency.name} is absent`,
      );
    }
    reachable.add(id);
    for (const dependencyId of node.dependencies) {
      const child = packagesById.get(dependencyId);
      if (child === undefined) {
        throw new Error(
          `dependency package ${dependencyId} is absent from Cargo packages`,
        );
      }
      if (!nodesById.has(dependencyId)) {
        throw new Error(
          `resolve node for dependency package ${child.name} is absent`,
        );
      }
      pending.push(dependencyId);
    }
  }

  return [...reachable]
    .map((id) => packagesById.get(id))
    .sort((left, right) => left.id.localeCompare(right.id, "en"));
}

export function cargoWorkspaceMemberPackageIds(metadata) {
  if (typeof metadata !== "object" || metadata === null) {
    throw new Error("cargo metadata is not an object");
  }
  const packages = requiredArray(metadata.packages, "packages");
  const packageIds = new Set(
    packages.map((dependency) => {
      if (typeof dependency !== "object" || dependency === null) {
        throw new Error("cargo metadata contains an invalid package");
      }
      return requiredString(dependency.id, "package id");
    }),
  );
  return requiredArray(metadata.workspace_members, "workspace members").map(
    (id) => {
      requiredString(id, "workspace member id");
      if (!packageIds.has(id)) {
        throw new Error(`workspace package ${id} is absent from Cargo packages`);
      }
      return id;
    },
  );
}

export function thirdPartyCargoDependencyClosure(
  metadata,
  rootPackageNames,
  firstPartyPackageIds = [],
) {
  const dependencies = cargoDependencyClosure(metadata, rootPackageNames);
  const firstPartyIds = new Set(cargoWorkspaceMemberPackageIds(metadata));
  for (const id of requiredArray(firstPartyPackageIds, "first-party package IDs")) {
    firstPartyIds.add(requiredString(id, "first-party package ID"));
  }
  return dependencies.filter(({ id }) => !firstPartyIds.has(id));
}
