#!/usr/bin/env node
//
// Generate THIRD-PARTY-LICENSES.md from the production Rust dependencies that
// ship in the release binary, so a binary-only distribution carries the notices
// its dependencies' licenses (MIT, Apache-2.0, …) require. Dev-only crates
// (compiled for `cargo test`/build tooling but never shipped) are excluded.
//
// The generated file is embedded into the binary (`include_str!`) and printed by
// `lisplens license`. Regenerate whenever the dependency tree changes:
//
//   node scripts/generate-third-party-licenses.mjs
//
// Method adapted from the riida project's generator (cargo metadata per release
// target → collect each crate's notice files → dedup the big shared license
// bodies, e.g. Apache-2.0, into an appendix).

import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, readFileSync, readdirSync, statSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const rootDir = path.resolve(__dirname, "..");
const outputFile = path.join(rootDir, "THIRD-PARTY-LICENSES.md");

// The release binary's platforms (see .github/workflows/release.yml). The union
// of each target's dependency graph is what ships across all published binaries.
const rustTargets = [
  "x86_64-unknown-linux-gnu",
  "aarch64-unknown-linux-gnu",
  "x86_64-apple-darwin",
  "aarch64-apple-darwin",
  "x86_64-pc-windows-msvc",
];

// Big license bodies that recur verbatim across many crates are collected once
// into an appendix and referenced, so the file doesn't repeat the full Apache
// text dozens of times.
const KNOWN_LICENSE_BODIES = [
  {
    title: "Apache-2.0",
    anchor: "apache-20",
    canonicalContentFile: path.join(rootDir, "licenses", "Apache-2.0.txt"),
    canonicalContentSha256: "cfc7749b96f63bd31c3c42b5c471bf756814053e847c10f3eb003417bc523d30",
    sha256s: new Set(),
    appendixOmittedSha256s: new Set(),
  },
];

function runWithResult(command, args, cwd = rootDir) {
  return spawnSync(command, args, {
    cwd,
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
    maxBuffer: 128 * 1024 * 1024,
  });
}

function cargoMetadata(target, { offline }) {
  const args = [
    "metadata",
    "--manifest-path",
    "Cargo.toml",
    "--format-version",
    "1",
    "--filter-platform",
    target,
  ];
  if (offline) {
    args.push("--offline");
  }
  const result = runWithResult("cargo", args, rootDir);
  if (result.status !== 0) {
    const error = new Error(result.stderr.trim() || `cargo ${args.join(" ")} failed`);
    error.stderr = result.stderr ?? "";
    throw error;
  }
  return JSON.parse(result.stdout);
}

function findNoticeFiles(packageDir) {
  if (!existsSync(packageDir)) {
    return [];
  }
  return readdirSync(packageDir, { withFileTypes: true })
    .filter((entry) => entry.isFile())
    .map((entry) => entry.name)
    .filter((name) => /^(license|licence|copying|notice)([._-].+)?$/i.test(name))
    .sort((a, b) => a.localeCompare(b));
}

function readNoticeFile(filePath) {
  if (statSync(filePath).size > 512 * 1024) {
    return "[omitted: notice file is larger than 512 KiB]";
  }
  return readFileSync(filePath, "utf8").trimEnd();
}

function normalizeNoticeContent(content) {
  return content.replace(/\r/g, "");
}

function noticeContentSha256(content) {
  return createHash("sha256").update(normalizeNoticeContent(content)).digest("hex");
}

function looksLikeApacheLicenseBody(content) {
  const c = normalizeNoticeContent(content);
  return (
    /^\s*Apache License\s*\n\s*Version 2\.0, January 2004/m.test(c) &&
    c.includes("TERMS AND CONDITIONS FOR USE, REPRODUCTION, AND DISTRIBUTION") &&
    c.includes("1. Definitions.") &&
    c.includes("2. Grant of Copyright License.") &&
    !c.includes("```")
  );
}

function resolveKnownLicenseBody(content) {
  const sha256 = noticeContentSha256(content);
  const match = KNOWN_LICENSE_BODIES.find((entry) => {
    if (entry.title === "Apache-2.0" && looksLikeApacheLicenseBody(content)) {
      return true;
    }
    return entry.sha256s.has(sha256);
  });
  if (!match) {
    return null;
  }
  const canonicalContent = normalizeNoticeContent(readFileSync(match.canonicalContentFile, "utf8"));
  const canonicalHash = noticeContentSha256(canonicalContent);
  if (match.canonicalContentSha256 && canonicalHash !== match.canonicalContentSha256) {
    throw new Error(
      `${match.title} canonical text hash mismatch: expected ${match.canonicalContentSha256}, got ${canonicalHash}`,
    );
  }
  return {
    title: match.title,
    anchor: match.anchor,
    appendixOmitted: match.appendixOmittedSha256s.has(sha256),
    content: canonicalContent,
  };
}

function formatPackageSection(packages, appendixEntries) {
  const lines = ["## Rust Dependencies", ""];
  for (const entry of packages) {
    lines.push(`### ${entry.name} ${entry.version}`, "");
    lines.push(`- License: ${entry.license}`);
    if (entry.authors) {
      lines.push(`- Authors: ${entry.authors}`);
    }
    if (entry.repository) {
      lines.push(`- Source: ${entry.repository}`);
    } else if (entry.homepage) {
      lines.push(`- Homepage: ${entry.homepage}`);
    }
    lines.push("");
    if (entry.noticeFiles.length === 0) {
      lines.push("_No local license or notice file was found in the installed package._", "");
      continue;
    }
    for (const noticeFile of entry.noticeFiles) {
      lines.push(`#### ${noticeFile.name}`, "");
      const known = resolveKnownLicenseBody(noticeFile.content);
      if (known) {
        appendixEntries.set(known.anchor, known);
        const label = `full text of ${known.title === "Apache-2.0" ? "the Apache License 2.0" : known.title}`;
        lines.push(
          known.appendixOmitted
            ? `_See the [${label}](#${known.anchor}). The original package license text omitted the APPENDIX section._`
            : `_See the [${label}](#${known.anchor})._`,
          "",
        );
        continue;
      }
      lines.push("```text", normalizeNoticeContent(noticeFile.content), "```", "");
    }
  }
  return lines;
}

function formatLicenseBodyAppendix(appendixEntries) {
  if (appendixEntries.size === 0) {
    return [];
  }
  const lines = ["## License body", ""];
  for (const entry of appendixEntries.values()) {
    lines.push(`### ${entry.title}`, "", "```text", entry.content, "```", "");
  }
  return lines;
}

// Package ids reachable from the workspace through normal/build edges (dev-only
// edges dropped) — the crates actually shipped in the release binary.
function computeProductionPackageIds(metadata) {
  const resolve = metadata.resolve;
  if (!resolve || !Array.isArray(resolve.nodes)) {
    return null;
  }
  const nodeById = new Map(resolve.nodes.map((node) => [node.id, node]));
  const roots =
    Array.isArray(metadata.workspace_members) && metadata.workspace_members.length > 0
      ? metadata.workspace_members
      : resolve.root
        ? [resolve.root]
        : [];
  const included = new Set();
  const stack = [...roots];
  while (stack.length > 0) {
    const id = stack.pop();
    if (included.has(id)) {
      continue;
    }
    included.add(id);
    const node = nodeById.get(id);
    if (!node || !Array.isArray(node.deps)) {
      continue;
    }
    for (const dep of node.deps) {
      const kinds = Array.isArray(dep.dep_kinds) ? dep.dep_kinds : [];
      const isProduction =
        kinds.length === 0 || kinds.some((k) => k.kind === null || k.kind === "build");
      if (isProduction && !included.has(dep.pkg)) {
        stack.push(dep.pkg);
      }
    }
  }
  return included;
}

function collectRustPackages() {
  const packages = new Map();
  for (const target of rustTargets) {
    let metadata;
    try {
      metadata = cargoMetadata(target, { offline: true });
    } catch (error) {
      const stderr = typeof error?.stderr === "string" ? error.stderr : "";
      const retry =
        stderr.includes("you're using offline mode") ||
        stderr.includes("no matching package named") ||
        stderr.includes("failed to download");
      if (!retry) {
        throw error;
      }
      metadata = cargoMetadata(target, { offline: false });
    }
    const productionIds = computeProductionPackageIds(metadata);
    for (const pkg of metadata.packages.filter((p) => p.source)) {
      if (packages.has(pkg.id) || (productionIds && !productionIds.has(pkg.id))) {
        continue;
      }
      const packageDir = path.dirname(pkg.manifest_path);
      const noticeFiles = findNoticeFiles(packageDir).map((name) => ({
        name,
        content: readNoticeFile(path.join(packageDir, name)),
      }));
      packages.set(pkg.id, {
        name: pkg.name,
        version: pkg.version,
        license: pkg.license ?? "UNKNOWN",
        authors:
          Array.isArray(pkg.authors) && pkg.authors.length > 0 ? pkg.authors.join(", ") : null,
        homepage: pkg.homepage ?? pkg.documentation ?? null,
        repository: pkg.repository ?? null,
        noticeFiles,
      });
    }
  }
  return [...packages.values()].sort(
    (a, b) => a.name.localeCompare(b.name) || a.version.localeCompare(b.version),
  );
}

const rustPackages = collectRustPackages();
const appendixEntries = new Map();
const output = [
  "# Third-Party Licenses",
  "",
  "lisplens itself is licensed under MPL-2.0 (see the top-level `LICENSE`). This",
  "file collects the license notices of the production Rust dependencies bundled",
  "into the release binary, so a binary-only distribution carries the notices those",
  "licenses require. Dev-only crates (test/build tooling, never shipped) are excluded.",
  "",
  `- Rust dependencies: ${rustPackages.length}`,
  "",
  "_Generated by `scripts/generate-third-party-licenses.mjs`; do not edit by hand._",
  "",
  ...formatPackageSection(rustPackages, appendixEntries),
  ...formatLicenseBodyAppendix(appendixEntries),
].join("\n");

writeFileSync(outputFile, `${output}\n`, "utf8");
console.log(`Wrote ${path.relative(rootDir, outputFile)} (${rustPackages.length} crates)`);
