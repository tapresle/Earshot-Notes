// Prints the license notices for the JavaScript packages that ship in the app.
//
// Shipped packages are the production dependency tree in package-lock.json
// (every package not marked `dev`). Dev dependencies such as Vite, ESLint and
// the test tools only run at build time and aren't bundled.
//
// Each package's license files are read from node_modules, so run `npm install`
// first. A package published without a license file can have one in
// scripts/notices/npm-fallback/<package name>/. The script fails if a shipped
// package has neither, or if node_modules doesn't match the lockfile.
//
// Usage: node scripts/notices/js-notices.mjs > notices.txt
import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const fallbackDir = join(root, "scripts/notices/npm-fallback");
const licenseFile = /^(licen[cs]e|copying|notice)([-_.].*)?$/i;

const lock = JSON.parse(readFileSync(join(root, "package-lock.json"), "utf8"));

function licenseFiles(dir) {
  if (!existsSync(dir)) return [];
  return readdirSync(dir)
    .filter((name) => licenseFile.test(name) && !name.endsWith(".spdx"))
    .filter((name) => statSync(join(dir, name)).isFile())
    .sort()
    .map((name) => join(dir, name));
}

const packages = Object.entries(lock.packages)
  .filter(([path, info]) => path !== "" && !info.dev)
  .map(([path, info]) => {
    const name = info.name ?? path.slice(path.lastIndexOf("node_modules/") + "node_modules/".length);
    return { path, name, version: info.version, license: info.license };
  })
  .sort((a, b) => a.name.localeCompare(b.name) || a.version.localeCompare(b.version));

const errors = [];
const sections = [];

for (const pkg of packages) {
  const dir = join(root, pkg.path);
  const manifest = join(dir, "package.json");
  if (!existsSync(manifest)) {
    errors.push(`${pkg.name} isn't installed; run \`npm install\``);
    continue;
  }
  const installed = JSON.parse(readFileSync(manifest, "utf8"));
  if (installed.version !== pkg.version) {
    errors.push(
      `${pkg.name} is ${installed.version} in node_modules but ${pkg.version} in package-lock.json; run \`npm install\``,
    );
    continue;
  }

  let files = licenseFiles(dir);
  if (files.length === 0) files = licenseFiles(join(fallbackDir, pkg.name));
  if (files.length === 0) {
    errors.push(
      `${pkg.name} ${pkg.version} has no license file; add one under scripts/notices/npm-fallback/${pkg.name}/`,
    );
    continue;
  }

  const repository =
    typeof installed.repository === "string" ? installed.repository : installed.repository?.url;
  const texts = files.map((file) => readFileSync(file, "utf8").trimEnd()).join("\n\n");
  sections.push(
    [
      "--------------------------------------------------------------------------------",
      `${pkg.name} ${pkg.version} (${pkg.license ?? installed.license ?? "unknown license"})`,
      ...(repository ? [repository] : []),
      "",
      texts,
      "",
    ].join("\n"),
  );
}

if (errors.length > 0) {
  for (const error of errors) console.error(`error: ${error}`);
  process.exit(1);
}

process.stdout.write(sections.join("\n"));
