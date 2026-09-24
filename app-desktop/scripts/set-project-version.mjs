import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";

const appRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const version = process.argv[2] || process.env.AI_VIDEO_LOCALIZATION_VERSION;
const semverPattern = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$/;

if (!version || !semverPattern.test(version)) {
  throw new Error("Provide a valid semantic version, for example 1.2.3 or 1.2.3-beta.1.");
}

function read(relativePath) {
  return readFileSync(path.join(appRoot, relativePath), "utf8");
}

function write(relativePath, contents) {
  writeFileSync(path.join(appRoot, relativePath), contents, "utf8");
}

function updateJson(relativePath, update) {
  const document = JSON.parse(read(relativePath));
  update(document);
  write(relativePath, `${JSON.stringify(document, null, 2)}\n`);
}

function updateSectionVersion(relativePath, sectionName) {
  const contents = read(relativePath);
  const marker = `[${sectionName}]`;
  const sectionStart = contents.indexOf(marker);
  if (sectionStart < 0) {
    throw new Error(`${relativePath} is missing [${sectionName}].`);
  }
  const nextSection = contents.indexOf("\n[", sectionStart + marker.length);
  const sectionEnd = nextSection < 0 ? contents.length : nextSection + 1;
  const section = contents.slice(sectionStart, sectionEnd);
  const updated = section.replace(/^(version\s*=\s*)"[^"]+"/m, `$1"${version}"`);
  if (updated === section) {
    throw new Error(`${relativePath} is missing a version in [${sectionName}].`);
  }
  write(relativePath, contents.slice(0, sectionStart) + updated + contents.slice(sectionEnd));
}

updateJson("package.json", (document) => { document.version = version; });
updateJson("package-lock.json", (document) => {
  document.version = version;
  if (document.packages?.[""]) document.packages[""].version = version;
});
updateJson("src-tauri/tauri.conf.json", (document) => { document.version = version; });
updateSectionVersion("src-tauri/Cargo.toml", "package");
updateSectionVersion("python/pyproject.toml", "project");

const workerVersionPath = "python/worker/_version.py";
const workerVersionText = read(workerVersionPath);
const updatedWorkerVersion = workerVersionText.replace(
  /^__version__\s*=\s*"[^"]+"/m,
  `__version__ = "${version}"`,
);
if (updatedWorkerVersion === workerVersionText) {
  throw new Error(`${workerVersionPath} is missing __version__.`);
}
write(workerVersionPath, updatedWorkerVersion);

let cargoLock = read("src-tauri/Cargo.lock");
const cargoPackagePattern = /(\[\[package\]\]\r?\nname\s*=\s*"ai-video-localization"\r?\nversion\s*=\s*)"[^"]+"/;
if (!cargoPackagePattern.test(cargoLock)) {
  throw new Error("src-tauri/Cargo.lock is missing the application package entry.");
}
cargoLock = cargoLock.replace(cargoPackagePattern, `$1"${version}"`);
write("src-tauri/Cargo.lock", cargoLock);

console.log(`Synchronized application and worker version to ${version}.`);
