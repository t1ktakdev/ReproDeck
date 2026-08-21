use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum DemoError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("the desktop folder is unavailable")]
    DesktopUnavailable,
    #[error("git could not initialize the demo fixture: {0}")]
    Git(String),
}

pub type Result<T> = std::result::Result<T, DemoError>;

const PACKAGE_JSON: &str = r#"{
  "name": "reprodeck-demo-fixture",
  "version": "1.0.0",
  "private": true,
  "type": "module",
  "description": "Deterministic ReproDeck evidence-loop fixture.",
  "scripts": {
    "check": "node scripts/check.js",
    "test": "node tests/integration.test.js",
    "build": "node scripts/build.js"
  }
}
"#;
const CACHE_KEY: &str = r#"export function cacheKey(tenant, userId) {
  // Intentional defect: tenant is ignored, so identical user IDs collide.
  return userId;
}
"#;
const TENANT: &str = r#"export function normalizeTenant(value) {
  return String(value).trim().toLowerCase();
}
"#;
const TOKEN_STORE: &str = r#"import { normalizeTenant } from "./tenant.js";
export class TokenStore {
  #tokens = new Map();
  write(tenant, userId, token) { this.#tokens.set(`${normalizeTenant(tenant)}:${userId}`, token); }
  read(tenant, userId) { return this.#tokens.get(`${normalizeTenant(tenant)}:${userId}`) ?? null; }
}
"#;
const AUTH_CACHE: &str = r#"import { cacheKey } from "./cache-key.js";
import { normalizeTenant } from "./tenant.js";
export class AuthCache {
  #values = new Map();
  get(tenant, userId) { return this.#values.get(cacheKey(normalizeTenant(tenant), userId)) ?? null; }
  set(tenant, userId, token) { this.#values.set(cacheKey(normalizeTenant(tenant), userId), token); }
  invalidate(tenant, userId) { this.#values.delete(`${normalizeTenant(tenant)}:${userId}`); }
}
"#;
const SERVICE: &str = r#"import { AuthCache } from "./auth-cache.js";
import { TokenStore } from "./token-store.js";
export class AuthorizationService {
  constructor(store = new TokenStore(), cache = new AuthCache()) { this.store = store; this.cache = cache; }
  seed(tenant, userId, token) { this.store.write(tenant, userId, token); }
  authorizationHeader(tenant, userId) {
    let token = this.cache.get(tenant, userId);
    if (token === null) { token = this.store.read(tenant, userId); if (token !== null) this.cache.set(tenant, userId, token); }
    return token === null ? null : `Bearer ${token}`;
  }
  refresh(tenant, userId, nextToken) { this.store.write(tenant, userId, nextToken); this.cache.invalidate(tenant, userId); }
}
"#;
const TEST: &str = r#"import { AuthorizationService } from "../src/authorization-service.js";
const failures = [];
const equal = (label, actual, expected) => { if (actual !== expected) failures.push(`AssertionError [${label}]: expected ${expected} but received ${actual}`); };
const service = new AuthorizationService();
service.seed("alpha", "user-42", "ALPHA-1");
service.seed("beta", "user-42", "BETA-1");
equal("alpha-initial", service.authorizationHeader("alpha", "user-42"), "Bearer ALPHA-1");
equal("tenant-isolation", service.authorizationHeader("beta", "user-42"), "Bearer BETA-1");
service.refresh("beta", "user-42", "BETA-2");
equal("refresh-invalidation", service.authorizationHeader("beta", "user-42"), "Bearer BETA-2");
if (failures.length) { console.error("Authorization integration suite failed."); failures.forEach(value => console.error(value)); console.error("Observed pattern: cached authorization state crosses a tenant boundary."); process.exit(1); }
console.log("authorization integration suite passed");
"#;
const CHECK: &str = r#"import { spawnSync } from "node:child_process";
import { readdirSync } from "node:fs";
import { join } from "node:path";
for (const file of [...readdirSync("src").map(name => join("src", name)), ...readdirSync("tests").map(name => join("tests", name))]) {
  const result = spawnSync(process.execPath, ["--check", file], { stdio: "inherit" });
  if (result.status !== 0) process.exit(result.status ?? 1);
}
console.log("syntax check passed");
"#;
const BUILD: &str = r#"import { existsSync } from "node:fs";
for (const file of ["src/authorization-service.js", "src/auth-cache.js", "src/cache-key.js", "src/token-store.js", "src/tenant.js"]) {
  if (!existsSync(file)) { console.error(`BuildError: missing ${file}`); process.exit(1); }
}
console.log("release-shape validation passed");
"#;
const README: &str = r#"# ReproDeck demo fixture

This dependency-free project contains a deterministic multi-file authorization bug.

- `npm run check` passes
- `npm test` fails with tenant isolation and refresh symptoms
- `npm run build` passes

Use ReproDeck to investigate evidence and test one minimal intervention. Do not edit the original repository during the investigation.
"#;

fn git(path: &Path, args: &[&str]) -> Result<()> {
    let output = Command::new("git").current_dir(path).args(args).output()?;
    if output.status.success() {
        Ok(())
    } else {
        Err(DemoError::Git(
            String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        ))
    }
}

fn available_target(desktop: &Path) -> PathBuf {
    let preferred = desktop.join("ReproDeck-Demo-Fixture");
    if !preferred.exists() {
        return preferred;
    }
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs())
        .unwrap_or_default();
    desktop.join(format!("ReproDeck-Demo-Fixture-{suffix}"))
}

/// Create a real, dependency-free Git fixture after the user clicks Try Demo.
/// Existing folders are never overwritten or deleted.
pub fn create_fixture() -> Result<String> {
    let desktop = directories::UserDirs::new()
        .and_then(|dirs| dirs.desktop_dir().map(Path::to_path_buf))
        .ok_or(DemoError::DesktopUnavailable)?;
    let target = available_target(&desktop);
    fs::create_dir(&target)?;
    let created = (|| -> Result<()> {
        for directory in ["src", "tests", "scripts"] {
            fs::create_dir(target.join(directory))?;
        }
        for (path, contents) in [
            ("package.json", PACKAGE_JSON),
            ("README.md", README),
            ("src/cache-key.js", CACHE_KEY),
            ("src/tenant.js", TENANT),
            ("src/token-store.js", TOKEN_STORE),
            ("src/auth-cache.js", AUTH_CACHE),
            ("src/authorization-service.js", SERVICE),
            ("tests/integration.test.js", TEST),
            ("scripts/check.js", CHECK),
            ("scripts/build.js", BUILD),
        ] {
            fs::write(target.join(path), contents.as_bytes())?;
        }
        git(&target, &["init"])?;
        git(&target, &["config", "user.name", "ReproDeck Fixture"])?;
        git(
            &target,
            &["config", "user.email", "fixture@reprodeck.invalid"],
        )?;
        git(&target, &["config", "core.autocrlf", "false"])?;
        git(&target, &["add", "."])?;
        git(
            &target,
            &["commit", "-m", "fixture: tenant authorization failure"],
        )?;
        Ok(())
    })();
    if let Err(error) = created {
        let _ = fs::remove_dir_all(&target);
        return Err(error);
    }
    Ok(target.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_sources_keep_the_intentional_failure_and_real_commands() {
        assert!(PACKAGE_JSON.contains("node tests/integration.test.js"));
        assert!(CACHE_KEY.contains("return userId"));
        assert!(TEST.contains("process.exit(1)"));
        assert!(!TEST.contains("mock"));
    }
}
