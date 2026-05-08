import { execSync, execFileSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const MANIFEST_DIR = dirname(fileURLToPath(import.meta.url));
const PROFILE = process.env.PROFILE || "debug";

// Cargo target-dir naming convention: "dev"/"test" → "debug", every
// other profile (including custom ones like `release-ci`) maps to a
// directory matching the profile name verbatim. Centralize the mapping
// so the source-dir lookup + the cargo CLI args stay in sync.
const PROFILE_DIR =
  PROFILE === "dev" || PROFILE === "test" ? "debug" : PROFILE;

// Translate profile name to cargo CLI args. `--release` is a legacy
// alias for `--profile release`; for any other custom profile we pass
// `--profile <name>` explicitly.
function profileToCargoArgs(profile) {
  if (profile === "debug" || profile === "dev") return [];
  if (profile === "release") return ["--release"];
  return ["--profile", profile];
}

function getTarget() {
  if (process.env.TARGET) return process.env.TARGET;
  try {
    const output = execSync("rustc -vV", { encoding: "utf-8" });
    const match = output.match(/host:\s*(.+)/);
    if (match) return match[1].trim();
  } catch {}
  return "unknown";
}

function getHostTarget() {
  try {
    const output = execSync("rustc -vV", { encoding: "utf-8" });
    const match = output.match(/host:\s*(.+)/);
    if (match) return match[1].trim();
  } catch {}
  return "unknown";
}

const TARGET = getTarget();
const HOST_TARGET = getHostTarget();
const isWindows = TARGET.includes("windows");

// Determine source directory using PROFILE_DIR (handles dev/test→debug
// and custom profiles like release-ci verbatim).
let srcDir;
if (TARGET === HOST_TARGET || TARGET === "unknown") {
  srcDir = join(MANIFEST_DIR, "target", PROFILE_DIR);
} else {
  srcDir = join(MANIFEST_DIR, "target", TARGET, PROFILE_DIR);
}

const destDir = join(MANIFEST_DIR, "binaries");
mkdirSync(destDir, { recursive: true });

function copyBinary(baseName) {
  const binName = isWindows ? `${baseName}.exe` : baseName;
  const source = join(srcDir, binName);

  let destName = `${baseName}-${TARGET}`;
  if (isWindows) destName += ".exe";
  const dest = join(destDir, destName);

  if (existsSync(source)) {
    copyFileSync(source, dest);
    console.log(`Copied ${binName} to ${dest}`);
  } else {
    console.log(`Warning: Binary not found at ${source}`);
    console.log(`Building ${baseName} binary...`);

    const buildArgs = ["build", "--bin", baseName, ...profileToCargoArgs(PROFILE)];
    if (TARGET !== "unknown" && TARGET !== HOST_TARGET) {
      buildArgs.push("--target", TARGET);
    }

    execFileSync("cargo", buildArgs, {
      cwd: MANIFEST_DIR,
      stdio: "inherit",
    });

    if (existsSync(source)) {
      copyFileSync(source, dest);
      console.log(`Built and copied ${binName} to ${dest}`);
    } else {
      console.error(`Error: Failed to build ${baseName} binary`);
      process.exit(1);
    }
  }
}

copyBinary("donut-proxy");
copyBinary("donut-daemon");
