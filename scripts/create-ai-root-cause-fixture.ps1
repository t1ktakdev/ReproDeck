param(
    [string]$Path = "$HOME\Desktop\ReproDeck-AI-RootCause-Fixture",
    [switch]$Force
)

$ErrorActionPreference = "Stop"

if (Test-Path $Path) {
    if (-not $Force) {
        throw "Fixture already exists: $Path. Re-run with -Force to recreate it."
    }
    Remove-Item -Recurse -Force $Path
}

New-Item -ItemType Directory -Force -Path $Path, "$Path\src", "$Path\tests", "$Path\scripts" | Out-Null

$Utf8NoBom = New-Object System.Text.UTF8Encoding($false)
function Write-Utf8NoBom([string]$Target, [string]$Content) {
    [System.IO.File]::WriteAllText($Target, $Content, $Utf8NoBom)
}

$packageJson = @'
{
  "name": "reprodeck-ai-root-cause-fixture",
  "version": "1.0.0",
  "private": true,
  "type": "module",
  "description": "Dependency-free multi-file fixture for evidence-bound AI root-cause investigation.",
  "scripts": {
    "check": "node scripts/check.js",
    "test": "node tests/integration.test.js",
    "build": "node scripts/build.js"
  }
}
'@
Write-Utf8NoBom "$Path\package.json" $packageJson

$cacheKey = @'
export function cacheKey(tenant, userId) {
  // Intentional defect: tenant is ignored, so identical user IDs collide across tenants.
  return userId;
}
'@
Write-Utf8NoBom "$Path\src\cache-key.js" $cacheKey

$tenant = @'
export function normalizeTenant(value) {
  return String(value).trim().toLowerCase();
}
'@
Write-Utf8NoBom "$Path\src\tenant.js" $tenant

$tokenStore = @'
import { normalizeTenant } from "./tenant.js";

export class TokenStore {
  #tokens = new Map();

  write(tenant, userId, token) {
    this.#tokens.set(`${normalizeTenant(tenant)}:${userId}`, token);
  }

  read(tenant, userId) {
    return this.#tokens.get(`${normalizeTenant(tenant)}:${userId}`) ?? null;
  }
}
'@
Write-Utf8NoBom "$Path\src\token-store.js" $tokenStore

$authCache = @'
import { cacheKey } from "./cache-key.js";
import { normalizeTenant } from "./tenant.js";

export class AuthCache {
  #values = new Map();

  get(tenant, userId) {
    return this.#values.get(cacheKey(normalizeTenant(tenant), userId)) ?? null;
  }

  set(tenant, userId, token) {
    this.#values.set(cacheKey(normalizeTenant(tenant), userId), token);
  }

  invalidate(tenant, userId) {
    // Invalidation follows the documented tenant-aware contract.
    this.#values.delete(`${normalizeTenant(tenant)}:${userId}`);
  }
}
'@
Write-Utf8NoBom "$Path\src\auth-cache.js" $authCache

$retry = @'
export function retryDelay(attempt) {
  const bounded = Math.max(0, Math.min(4, Number(attempt) || 0));
  return 25 * (2 ** bounded);
}
'@
Write-Utf8NoBom "$Path\src\retry-policy.js" $retry

$service = @'
import { AuthCache } from "./auth-cache.js";
import { TokenStore } from "./token-store.js";

export class AuthorizationService {
  constructor(store = new TokenStore(), cache = new AuthCache()) {
    this.store = store;
    this.cache = cache;
  }

  seed(tenant, userId, token) {
    this.store.write(tenant, userId, token);
  }

  authorizationHeader(tenant, userId) {
    let token = this.cache.get(tenant, userId);
    if (token === null) {
      token = this.store.read(tenant, userId);
      if (token !== null) this.cache.set(tenant, userId, token);
    }
    return token === null ? null : `Bearer ${token}`;
  }

  refresh(tenant, userId, nextToken) {
    this.store.write(tenant, userId, nextToken);
    this.cache.invalidate(tenant, userId);
  }
}
'@
Write-Utf8NoBom "$Path\src\authorization-service.js" $service

$integrationTest = @'
import { AuthorizationService } from "../src/authorization-service.js";
import { retryDelay } from "../src/retry-policy.js";

const failures = [];
function expectEqual(label, actual, expected) {
  if (actual !== expected) {
    failures.push(`AssertionError [${label}]: expected ${expected} but received ${actual}`);
  }
}

const service = new AuthorizationService();
service.seed("alpha", "user-42", "ALPHA-TOKEN-1");
service.seed("beta", "user-42", "BETA-TOKEN-1");
service.seed("beta", "user-99", "BETA-OTHER-1");

expectEqual(
  "alpha-initial",
  service.authorizationHeader("alpha", "user-42"),
  "Bearer ALPHA-TOKEN-1",
);

expectEqual(
  "tenant-isolation",
  service.authorizationHeader("beta", "user-42"),
  "Bearer BETA-TOKEN-1",
);

service.refresh("beta", "user-42", "BETA-TOKEN-2");
expectEqual(
  "refresh-invalidation",
  service.authorizationHeader("beta", "user-42"),
  "Bearer BETA-TOKEN-2",
);

expectEqual(
  "unrelated-user",
  service.authorizationHeader("beta", "user-99"),
  "Bearer BETA-OTHER-1",
);

expectEqual("retry-policy-decoy", retryDelay(2), 100);

if (failures.length > 0) {
  console.error("Authorization integration suite failed.");
  for (const failure of failures) console.error(failure);
  console.error("Observed pattern: same userId appears in multiple tenants; cached authorization state crosses a tenant boundary.");
  process.exit(1);
}

console.log("authorization integration suite passed");
'@
Write-Utf8NoBom "$Path\tests\integration.test.js" $integrationTest

$check = @'
import { spawnSync } from "node:child_process";
import { readdirSync } from "node:fs";
import { join } from "node:path";

const files = [
  ...readdirSync("src").filter(name => name.endsWith(".js")).map(name => join("src", name)),
  ...readdirSync("tests").filter(name => name.endsWith(".js")).map(name => join("tests", name)),
];
for (const file of files) {
  const result = spawnSync(process.execPath, ["--check", file], { stdio: "inherit" });
  if (result.status !== 0) process.exit(result.status ?? 1);
}
console.log(`syntax check passed for ${files.length} files`);
'@
Write-Utf8NoBom "$Path\scripts\check.js" $check

$build = @'
const required = [
  "src/authorization-service.js",
  "src/auth-cache.js",
  "src/cache-key.js",
  "src/token-store.js",
  "src/tenant.js",
  "src/retry-policy.js"
];
import { existsSync } from "node:fs";
for (const file of required) {
  if (!existsSync(file)) {
    console.error(`BuildError: missing required module ${file}`);
    process.exit(1);
  }
}
console.log("release-shape validation passed");
'@
Write-Utf8NoBom "$Path\scripts\build.js" $build

$readme = @'
# ReproDeck AI Root Cause fixture

This project contains a deterministic multi-file authorization bug.

Expected baseline:
- `npm run check` -> PASS
- `npm test` -> FAIL with tenant isolation / refresh symptoms
- `npm run build` -> PASS

Several files are plausible suspects and the failing output describes symptoms rather than naming the defective implementation. Use evidence relationships and one minimal causal intervention to distinguish the root cause from the false leads.

Do not edit the original repository during an Investigation Case. Make the intervention only inside ReproDeck's Fix Workspace.
'@
Write-Utf8NoBom "$Path\README.md" $readme

Push-Location $Path
try {
    git init | Out-Null
    git config user.name "ReproDeck Fixture"
    git config user.email "fixture@reprodeck.invalid"
    git config core.autocrlf false
    git add .
    git commit -m "fixture: multi-tenant authorization failure" | Out-Null

    Write-Host "Created ReproDeck demo fixture: $Path" -ForegroundColor Green
    Write-Host "Running baseline checks..." -ForegroundColor Cyan
    npm run check
    npm test
    if ($LASTEXITCODE -eq 0) {
        throw "Fixture test unexpectedly passed; the intentional bug is missing."
    }
    npm run build
    Write-Host "Baseline is ready: check PASS / test FAIL / build PASS" -ForegroundColor Green
} finally {
    Pop-Location
}

Write-Host "Important: the original fixture must remain unchanged during the causal experiment." -ForegroundColor Yellow
